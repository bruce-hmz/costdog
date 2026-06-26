# CostDog 改进计划（最终版）

> 本文 = 我对仓库实际代码的审核 + ChatGPT 路线图的适配性裁决，合并成一份可执行计划。
> 原则：**基于真实代码，不做应声虫。** ChatGPT 的方案有 ~60% 不匹配当前架构，照搬会毁掉 CostDog 真正的差异化。

---

## 0. 结论先行（TL;DR）

**CostDog 的真实身份不是 "AI Cost Intelligence Platform"，而是：**

> **本地优先、零配置、隐私安全的个人 AI CLI 花费监控器** —— 自动读取你机器上的 Claude Code / Codex 会话日志，算出花了多少钱、烧了多少 token、写了多少磁盘。

这恰恰是 ChatGPT 完全没看到的护城河：

- **零配置**：不用填 API Key、不用挂代理、不用改一行代码，装上就能看。
- **隐私**：数据全在本地 SQLite，不上传任何第三方（价格查询走 OpenRouter 公开 API）。
- **真实花费**：直接读 CLI 实际产生的 usage，比任何"按 API Key 估算"都准。

**我的核心建议：守住"本地优先"定位作为楔子，把 ChatGPT 方案里"能在本地落地"的好想法（预算/预测/模型替换省钱建议/AI 周报/更丰富的排行与趋势）吸收进来；把"必须变成云原生多租户 API 网关"的部分（管所有 Provider、API Key 治理、团队/RBAC、自动禁用 Key、企业版）明确否决，或拆成另一个产品线，不要污染当前产品。**

---

## 1. 我对当前产品的代码级审核（实测，非推测）

按严重度排序，都带 `文件:行号` 证据。

### 🔴 P0 真实 Bug

**1.1 告警会无限刷屏 + 撑爆数据库**
- 位置：`src/aggregator.ts` → `checkAlerts()` + `src/db/schema.ts` → `addAlert()`
- 问题：后台每 30 秒 `fullScan()` 一次（`src-tauri/src/lib.rs` 的 `std::thread::spawn` loop；TS 侧 `scan` 命令也会触发）。只要当天花费 > $10，**每次扫描都 INSERT 一条新告警**。一天下来 ~2880 条重复 "Daily cost exceeds $10"。`alerts` 表无去重、无 TTL。
- 修复：用 `(level, message, date)` 唯一约束 / 或单独的 `alert_state` 表记录"今天这条已报过"，只插一次。

**1.2 成本被系统性低估（Claude Code 尤其严重）**
- 位置：`src/utils/pricing.ts` → `calculateCost()`
- 问题 A：**完全没算 `cacheCreationTokens`**。Anthropic 的 cache 写入是 **1.25× input 价**，是 Claude Code 主要成本之一。schema 里有这个字段（`cache_creation_tokens`），解析器也采了（`claude-code.ts` 的 `cache_creation_input_tokens`），但算钱时被丢弃 → Claude Code 花费被低估。
- 问题 B：**reasoning tokens 没计入成本**。Codex 的 `reasoning_output_tokens`（o 系列/思考模型按 output 计费）单独存着，没进 `calculateCost`。
- 问题 C：cache read 写死 `0.1× input`，对 Anthropic 是对的，但其他厂商未必；价格表里 `cacheReadPricePerMToken` 字段已定义却没用上。
- 修复：补 cache creation（1.25×）、补 reasoning（按 output 价）、用价格表里的 cacheRead 价（缺省回落 0.1×）。

**1.3 macOS 桌面 bar 关闭后无法恢复、无法退出**
- 位置：`src-tauri/src/lib.rs` → `close_window()`（`#[cfg(target_os = "macos")] window.hide()`）+ `Cargo.toml` 启用了 `tray-icon` feature 但 `Builder` 里**没有 tray 设置**。
- 问题：点 ✕ = `hide()`，窗口消失但进程还在跑，**没有托盘图标**可以唤回，也没有"退出"入口。用户只能 kill 进程重开。
- 修复：补一个 tray icon（menu: Show / Scan / Quit）；`close_window` 在 mac 上要么走 tray 隐藏、要么直接 `app.exit(0)`，二选一并明确语义。

### 🟠 P1 架构/正确性

**1.4 两套并行的日志解析实现，迟早漂移**
- TS 侧：`src/parsers/claude-code.ts`、`src/parsers/codex.ts`、`src/aggregator.ts`
- Rust 侧：`src-tauri/src/lib.rs` 自己又写了一套 `parse_json_file` / `scan_claude_sessions` / `scan_codex_sessions` / `full_scan`（行 159/181/311/405）
- 问题：两边写同一个 SQLite。任何解析规则变化（新字段、subagent 处理、token 口径）都要改两处，bar 显示和 web/CLI 显示会偷偷不一致。bar 刚修的拖拽 bug 就是这种"两套前端"味道的冰山一角。
- 修复方向（二选一）：① bar 改成调用本地 `costdog` CLI / Express（:3456）拿数据，Rust 不再自己解析；② 或反过来，把解析下沉成 Rust 核心库，TS 侧 shell out。**推荐①**，因为 TS 解析已是事实标准、且 web 端在用。

**1.5 全量重扫，无增量**
- 位置：`scanClaudeSessions()` / Rust `scan_claude_sessions()` 都是 `walkDir` 全量 `readFileSync` 每个文件。
- 问题：每 30 秒把历史所有 JSONL 整体重读重解析。重度用户几个月后日志上 GB，CPU/IO 会肉眼可见地飙。
- 修复：用 `(path, mtime, size)` 做指纹表，只解析变化过的文件；session 级用现有 `scanned_at` + `end_time` 判定是否还需更新。

**1.6 价格模糊匹配过于激进，可能静默错配**
- 位置：`findModelPrice()` 最后两轮是双向 `includes`。
- 问题：短 id 容易误命中（例如 `gpt-4` 之类会匹配到第一个包含它的条目），产生**静默错误成本**。
- 修复：收紧匹配层级 + 命中时记录"matched_via"，匹配不到时在 UI 明确标"未定价"，不要默默按 0 元。

**1.7 死代码 / 静默吞错（违反你自己 CLAUDE.md 的 Fail Fast）**
- `daily_cache` 表两张嘴都没人写没人读（TS/Rust 都确认无用）→ 删或用起来。
- 大量 `catch {}` 静默吞（`pricing.ts` 的 fetch 失败、解析器逐行 parse 失败）。逐行容错可以，但 fetch 失败应在 UI/日志体现，不要默默回落。

### 🟡 P2 数据源与口径

**1.8 只支持 Claude Code + Codex 两个源**
- README/CLAUDE.md 里自己列了想加：OpenCode、Trae。社区里还有 Gemini CLI、Cursor、Aider、Cline、Roo 等。
- Claude Code 解析器目前**跳过 subagents 目录**（`if (entry.name === 'subagents') continue`），这部分花费用 `agentId` 是可以单独归因的——正好对接 ChatGPT 想要的"按 Agent 看花费"，且数据现成。

---

## 2. 对 ChatGPT 方案的适配性裁决

| ChatGPT 提议 | 裁决 | 理由 |
|---|---|---|
| 3 问框架：发生了什么 / 为什么 / 接下来怎么办 | ✅ **采纳** | 作为 dashboard 信息架构，非常好 |
| Dashboard：今日/昨日/周/月/自定义 + 趋势图 | ✅ **采纳** | 数据都有，只差图表层 |
| 按维度统计（model / project / agent / source） | ✅ **采纳** | 库表已支持；agent 用 subagent 归因 |
| Top 排行（model / project / session） | ✅ **采纳** | 已有 topModels，扩展即可 |
| Budget + 月底预测 + 超阈值告警 | ✅ **采纳（本地）** | 纯本地可做，价值高、成本低 |
| 模型替换省钱建议（Sonnet→Flash 省 X%） | ✅ **采纳** | 价格数据现成，杀手级本地特性 |
| AI Insight / 周报 / Ask CostDog | 🟡 **改造后采纳** | 用**用户自带 LLM key** 调用，数据只发往用户选定的模型，保持本地优先 |
| UI 向 Linear/Raycast/Vercel 靠拢 | ✅ **采纳** | 当前 bar+面板确实粗糙 |
| 官网 + 免费工具（token 计算器/价格对比）做 SEO | ✅ **采纳** | 复用 `pricing.ts`，很好的入口 |
| "统一管理所有 AI Provider / API Key 分析 / 自动禁用 Key / 限流 / 降级模型" | ❌ **否决** | 需要 CostDog 变成 API 网关/代理。它是读日志的，不是扛流量的。要做请单开产品 |
| 团队 / Workspace / RBAC / Org / Audit Log / SSO | ❌ **否决（当前产品）** | 本地单机工具看不到"别人"。这是另一个"云团队版"产品 |
| 按用户排行（Alice/Bob 花了多少） | ❌ **否决** | 同上，单机日志无多用户身份 |
| Prompt Analytics：哪个 prompt 最贵 | 🟡 **重定义** | CLI 日志没有 prompt 模板名。改为"按项目/会话/Agent 维度的成本"，不要叫 prompt |
| Provider latency / error / retry 分析 | 🟡 **部分** | 日志里延迟数据很少；error 勉强。先做 error/retry，latency 标 future |
| Free/Pro/Team/Enterprise 定价分层 | ⏸ **冻结** | 除非决定做云版本，否则现在谈分层太早。本地工具先做开源 + 可选付费云同步 |

---

## 3. 最终分阶段计划

每个阶段都有明确验收标准（对应你 CLAUDE.md 的 Goal-Driven）。

### Phase 0 — 地基修复（1~2 天，先做）
> 不修这些，后面的功能都建在沙子上。

- [ ] 修告警去重（1.1）→ 验收：当天 >$10 时全天只产生 1 条告警
- [ ] 修成本口径：cache creation ×1.25、reasoning 按 output、用价格表 cacheRead（1.2）→ 验收：拿一个已知 Claude Code 会话手算 vs CostDog，误差 < 2%
- [ ] mac tray + 退出语义（1.3）→ 验收：bar 可最小化到托盘并唤回；有明确 Quit
- [ ] 删/用 `daily_cache`；fetch 失败要在 UI 可见（1.7）
- [ ] **顺带**：把刚修的 `data-tauri-drag-region="deep"` 合入（PR #1 已就绪）

### Phase 1 — 准确性 & 数据源（3~5 天）
> "看得全、算得准"。

- [ ] 统一解析实现（1.4）：bar 走本地 HTTP，Rust 不再自己解析 → 验收：改一处解析，bar 和 web 同步生效
- [ ] 增量扫描（1.5）：指纹表 `(path,mtime,size)` → 验收：二次扫描耗时从 O(全量) 降到 O(增量)，1 万会话时 < 500ms
- [ ] 收紧价格匹配 + 未定价显式标记（1.6）
- [ ] 加数据源：Gemini CLI、Cursor、OpenCode（CLAUDE.md 已规划）→ 验收：每加一个源，能在 dashboard 按 source 拆分
- [ ] subagent 归因：不再跳过，按 `agentId` 聚合（1.8）→ 验收：能看到"哪个 subagent 最烧钱"

### Phase 2 — 本地分析（核心价值，1~2 周）
> 对应 ChatGPT 的"发生了什么 + 为什么"，但全部本地落地。

- [ ] 趋势图：spend / token / sessions / disk 的日周月折线（数据都在 SQLite）
- [ ] 维度排行：model / project / source / agent 的 Top N 成本与 token
- [ ] **Budget & Forecast**：设月预算 → 实时进度条 + 按当前 burn rate 预测月底是否超支 → 这是个人/小团队最痛的点
- [ ] **模型替换省钱建议**：基于真实用量，算"如果把 Sonnet 换 Flash / GPT-5 换 mini 会省多少"，给百分比和绝对值
- [ ] 自定义时间范围 + 昨日对比

### Phase 3 — 智能（差异化，1~2 周）
> 对应"接下来怎么办"，且不破坏本地优先。

- [ ] AI 周报/日报：用**用户自己的 LLM key**（OpenAI/Claude/Gemini/本地 Ollama），只把聚合后的数字发给它生成文字总结
- [ ] Ask CostDog：自然语言问"这个月哪个项目涨最多"，转 SQL/聚合查询作答
- [ ] 异常归因：cost 环比上涨时，自动分解到 model/project/agent 的贡献度（"涨的 38% 里，GPT-5 占 30%"）
- [ ] 关键设计：LLM 调用 100% 可关、可换本地模型、默认不上传明细

### Phase 4 — 打磨 & 增长（持续）
- [ ] UI 重设计：bar 与 dashboard 向 Linear/Raycast 调性靠拢（留白、大数字、玻璃卡、柔和动效）
- [ ] 官网：Hero / Problem / Solution / Features / Dashboard Preview / Pricing(future) / Docs / FAQ / CTA
- [ ] 免费工具引流（复用 `pricing.ts`）：Token Cost Calculator、LLM Pricing Comparison、Model Savings Estimator
- [ ] 打包分发：Mac（已修拖拽/补 tray 后）+ Win + Linux，自动更新

---

## 4. 需要你拍板的关键决策

1. **产品边界**：是否同意"本地优先"为绝对主线，把"云团队版"作为**未来可选**的另一条线（而不是现在就往多租户靠）？
   - 我的推荐：是。本地优先是 CostDog 唯一不被 Helicone / Langfuse / Vercel AI Gateway 秒杀的差异化。
2. **AI 功能的隐私底线**：Phase 3 的 LLM 调用，默认走"用户自带 key"，还是你提供托管？
   - 我的推荐：默认自带 key / 本地 Ollama，托管做成可选 opt-in。
3. **解析统一方向**（1.4）：bar 调本地 CLI/HTTP（推荐①），还是把解析下沉到 Rust（②）？
   - 我的推荐：①，最快收敛、复用现有 TS 解析的事实标准。

---

## 5. 一页纸 Roadmap

```
P0 地基(1-2d)  → P1 准确&数据源(3-5d) → P2 本地分析(1-2w) → P3 智能(1-2w) → P4 打磨增长(持续)
告警去重          统一解析                趋势图                AI 周报(自带key)      UI 重设计
成本口径修正      增量扫描                维度排行              Ask CostDog           官网
mac tray/退出    更多 CLI 源             Budget+预测           异常归因              免费工具引流
                 subagent 归因           模型替换省钱建议                            自动更新
```

**北极星指标**：一个 Claude Code 重度用户装上后，5 秒内看到"今天花了多少、哪个项目最烧、能不能省"——并且数字准到能对账。
