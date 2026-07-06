# CostDog 消费类型分类（Activity Category）— 设计文档

> 日期：2026-07-06
> 状态：**v2.1，Codex 复审通过（原 4 阻断全解决，3 处新发现已修），可进入实现**
> 作者：bruce + Claude（brainstorming）
> 关联：`docs/PRODUCT_PLAN.md` Phase 2「维度排行」的延伸 —— 新增「活动类型」轴

## 修订记录

- **v1（初稿）**：brainstorming 共创，三节设计经用户逐节认可。
- **v2（本版）**：经 Codex 对照真实代码审核（4 阻断 / 7 重要 / 4 建议，结论「需修订后复审」），逐条修订。处置见 §14。主要变化：`classify` 加 `source` 入参、`byCategory` 下沉到每个 `DailySummary`、Rust 补采 `usage.server_tool_use`、tool_calls 按 day bucket 统计、Edit 判定提到 Write 之前、`explore` 补数值阈值、gitBranch 规范化、双迁移并发与 TS upsert 语义明确、RecentSession 补字段。
- **v2.1（本次）**：Codex 复审确认原 4 阻断【全部已解决】。修 3 处复审新发现：① category SQL 用 `COALESCE(NULLIF(...),'other')` 把 NULL/空归 other（不再过滤）；② `tool_calls` 列按 `source` 写 NULL/JSON，与三态语义一致；③ 双迁移连接补 `busy_timeout` + `SQLITE_BUSY` 重试。

---

## 1. 背景与目标

CostDog 现有统计维度全部是「按实体」：model / project / source / agent（`PRODUCT_PLAN.md` Phase 2）。这些回答「在哪、用啥」，但不回答 **「token 拿来干了啥」**。本功能新增一条 **「按活动性质」** 的轴，做成记账本式的「消费类型」分类，让用户一眼看到「今天的钱花在了写代码 / 调试 / 调研 / 问答 …」。

**北极星**：用户点开桌面 bar 的 detail 面板，5 秒内看到圆环 + 占比条，知道「token 花在哪类事上」。

**产品定位约束**：必须保持 CostDog「本地优先、零配置、隐私安全」定位（`PRODUCT_PLAN.md` §0）。分类不引入云调用、不需要 API Key、不上传任何数据。

**用户场景假设**：用户可能 **只挂 bar、从不跑 CLI/网页**。因此分类必须在 bar 自己的 Rust 扫描链路里完成，不能依赖 TS 扫描（否则只挂 bar 的用户永远看不到分类）。

---

## 2. 用户故事

- 作为 Claude Code 重度用户，我想看到「今天 40% 的钱花在写新功能、25% 在修 Bug」。
- 我想点开任一 session，看到它被归到某类的 **理由**（哪个工具/分支触发了规则），不是黑箱。
- 我想切换 Today / 7D / 30D / All，看不同周期的类别构成。

可解释性的落点：`RecentSession` 加 `activity_category` 字段（§8.1 改动点 7），前端列表每行带类别 emoji；点击展开的具体 reason 留 v2.x。

---

## 3. 非目标（YAGNI 边界，本轮不做）

- ❌ TS 侧（Web 面板 / CLI dashboard）的分类展示 —— 随架构统一（§12）再补。
- ❌ LLM 语义分类 —— 违背本地优先。
- ❌ bar 横条常驻区显示分类 —— 空间局促，留 v2。
- ❌ 用户自定义类别 / 规则 —— v1 硬编码。
- ❌ 提取文件扩展名识别「文档」类 —— 留 v2。
- ❌ 增量扫描、统一 TS/Rust 解析 —— 属 `PRODUCT_PLAN.md` P1，不在本轮。

---

## 4. 分类体系（9 类）

类别表是单一事实来源，Rust 分类引擎与 bar 前端共用。每类互斥、可解释。

| key | emoji | 中文名 | Catppuccin Mocha 色 | 主判定信号 |
|---|---|---|---|---|
| `feature` | ✨ | 新功能编码 | `#89b4fa` | Write 占比高，或 `feature/`/`feat/` 分支 |
| `bugfix` | 🐛 | 修 Bug | `#f38ba8` | Edit 为主且 Edit>Write，或 `fix/`/`bugfix/`/`hotfix/`/`patch/` 分支 |
| `refactor` | ♻️ | 重构 | `#f9e2af` | `refactor/` 分支（无分支信号时并入 feature） |
| `docs` | 📚 | 文档 | `#a6e3a1` | `docs/` 分支（无分支信号时并入 feature） |
| `research` | 🔍 | 调研问答 | `#cba6f7` | WebSearch/WebFetch（来自 `usage.server_tool_use`）多，或工具总数 0（仅 claude/codex） |
| `debug` | 🧪 | 调试执行 | `#fab387` | Bash 占主导 |
| `agent` | 🤖 | 智能体编排 | `#94e2d5` | Task/Agent 类工具占比高 |
| `explore` | 📖 | 代码阅读 | `#74c7ec` | Read/Grep/Glob 为主，且 Write+Edit ≤ 0.20 |
| `other` | ⚪ | 其他 | `#6c7086` | 兜底；opencode/zcode 无工具细节 |

> **关键修正（Codex 阻断①③）**：`research` 的 WebSearch/WebFetch 信号来自 Claude 日志的 `usage.server_tool_use.web_search_requests / web_fetch_requests`（**不是** `message.content[].tool_use`），Rust 采集时必须额外读这个字段并映射成工具计数（§8.1 改动点 3）。`opencode/zcode` 永远没有工具细节，由 `source` 入参直接归 `other`。

---

## 5. 判定规则链

`classify` 是纯函数，**v2 增加 `source` 入参**：

```
classify(tool_calls: HashMap<String,u64>, git_branch: Option<&str>, source: &str) -> &'static str
  total = sum(tool_calls.values)
  has_tool_detail = source ∈ { "claude-code", "codex" }   // opencode/zcode 无工具细节

  ① 规范化 git_branch 后取首段匹配（最可信的用户标签）
     规范化 = trim → lowercase → 去掉 "refs/heads/"、"origin/" 前缀，再按 "/" 切取第一段
     首段精确匹配关键词集合:
       fix | bugfix | hotfix | patch   → "bugfix"
       docs                             → "docs"
       refactor                         → "refactor"
       feat | feature                   → "feature"
     // main / develop / master / spec / chore / feat-category(无斜杠) 等不命中，落到 ②

  ② 工具占比主导（ratio = count / total）
     !has_tool_detail                  → "other"     // opencode/zcode:根本没采到工具
     total == 0                        → "research"  // claude/codex 纯对话
     ratio(Task + Agent)         > 0.40 → "agent"
     ratio(WebSearch + WebFetch) > 0.40 → "research"
     ratio(Bash)                 > 0.50 → "debug"
     ratio(Read+Grep+Glob)       > 0.60 → "explore"   // 且 ratio(Write+Edit) ≤ 0.20
     ratio(Edit)                 > 0.40 → "bugfix"    // 且 Edit > Write  ← 提到 Write 前
     ratio(Write)                > 0.30 → "feature"

  ③ 兜底                               → "other"
```

**v2 规则顺序变化的理由**：
- `!has_tool_detail` 放最前 —— Codex 阻断①：避免 opencode/zcode 的 `{}` 命中 `total==0 → research`。
- `Edit` 判定 **提到 `Write` 之前** 且加 `Edit > Write` —— Codex 重要③：避免 `{Write:4, Edit:6}` 错误命中 feature（它更像 bugfix）。
- `explore` 的"写低"给数值 `ratio(Write+Edit) ≤ 0.20` —— Codex 重要④：可测试。
- gitBranch 规范化 —— Codex 重要②：`origin/feature/x`、`refs/heads/fix/y` 才不会漏。

**阈值是 v1 经验值**（用户已确认不改），需真实数据校准 → `classify` 必须有单测（§10）。

---

## 6. 架构决策：本轮落在 Rust 侧

### 6.1 现状事实（代码级证据）

桌面 bar 是 **纯 Rust 链路的 Tauri 独立应用**，不经 TS：

- `src-tauri/src/lib.rs` 自带完整一套：`scan_*_sessions`（L327/483/607/734）/ `ensure_db_exists`（L177）/ `upsert_session`（L1049）/ `get_data`（L1239）/ `full_scan`（L1118）。
- bar 前端 `src-tauri/embedded/index.html` 通过 `window.__TAURI_INTERNALS__.invoke('get_data')` 拿数据。
- TS 侧（`src/aggregator.ts` + `src/web/server.ts`）只服务 CLI / 网页，bar 不用它。
- Rust 与 TS **写同一个 SQLite**（`~/.costdog/costdog.sqlite`），各自独立建表/迁移 —— `PRODUCT_PLAN.md` §1.4「两套解析实现」。

缺口：Rust 的 `scan_claude_sessions`（L327）**不解析 tool_use**（只读 assistant 的 token usage，L437–458），`SessionData`（L78）无 toolCalls 字段。TS 的 `parsers/claude-code.ts` 在算 toolCalls（L156）但 `aggregator.upsertSession` 不传它 → 解析完丢弃。

### 6.2 决策：Rust 补全（经 Codex 复审后确认）

候选三选一（Codex 阻断④重新裁决）：
1. **Rust 补全**（选定）—— bar 自动扫即有分类。
2. Codex 折中（TS 填分类、Rust 只查）—— 分类逻辑只一份，但 **bar 自动扫到的新 session 分类列空**，只挂 bar 不跑 CLI 的用户永远看不到分类。
3. 统一架构（bar 调 TS）—— 最干净但 bar 数据链路大重构，scope 远超「加分类」。

**选 1 的决定性理由（用户拍板）**：必须假设「用户只挂 bar、从不跑 CLI」。折中方案对这类用户完全无分类数据，违背 bar 的核心卖点。Rust 解析链路**本已存在**（已在 walk 日志、读 assistant message），本轮只是 **在既有解析循环里多采两个字段**（`tool_use` + `server_tool_use`），不是从零新写 parser；`classify` 是纯函数、有单测。

**诚实标注**：这是 **明知违反 `PRODUCT_PLAN.md` §1.4 推荐方向的短期产品押注**，会让两套实现的债小幅加重（新增一个 `classify` 函数 + 两个采集点）。真正消解靠 §12 的架构统一。Codex 提的折中方案在「统一架构完成前」都不适用，因为 bar 独立性优先级 > 代码重复成本。

### 6.3 不做跨语言共享配置

类别表只有 Rust 一处用 → 本轮定义在 Rust 侧（常量数组）。TS 侧做分类时（§12 之后）再抽 `categories.json` 共享。YAGNI。

---

## 7. 数据模型变更

`sessions` 表新增 3 列（Rust `ensure_db_exists` + TS `schema.ts` 都要加，**列名/类型/默认值两边完全一致**）：

| 列 | 类型 | 默认 | 说明 |
|---|---|---|---|
| `activity_category` | `TEXT` | `NULL` | `classify` 结果，9 类 key；NULL/空 = 未分类（UI 归 other） |
| `tool_calls` | `TEXT` | `NULL` | JSON：`{"Write":3,"Bash":8}` |
| `git_branch` | `TEXT` | `NULL` | 仅 claude-code |

**`tool_calls` 列的空值语义（Codex 建议③ + 复审新发现②）**：
- `NULL` = 该源无工具细节（opencode / zcode）
- `'{}'` = 有细节但没调用任何工具（claude/codex 纯对话）
- `'{"Write":..}'` = 正常
- **写入规则**：`upsert_session` 按 `source` 决定 —— opencode/zcode 写 `NULL`，claude/codex 写 `serde_json::to_string(&map)`（空 map 自然落 `'{}'`）。不能统一序列化，否则 opencode/zcode 会错写成 `'{}'`、丢失「无细节」语义。

**为什么存原始 `tool_calls`**：① 可解释（UI 展示工具构成）；② 可重算（改规则后一条 SQL 用原始值重算 category，不必重扫日志）。

**迁移并发安全（Codex 重要⑤ + 复审新发现③）**：Rust 与 TS 都对同一 DB 做迁移，真实代码当前两边写连接都没设 `busy_timeout`（`lib.rs:177-185`、`schema.ts:13-15`），并发 ALTER 会先撞 `SQLITE_BUSY` 而不是 duplicate column。要求：
- 两边写连接都设 `busy_timeout=5000`（Rust `PRAGMA busy_timeout=5000`、TS `db.pragma('busy_timeout = 5000')`）。
- 新增 3 列各写一个 `pragma_table_info` 存在性检查 + `ALTER TABLE ADD COLUMN`，放在事务里；对「duplicate column」错误做幂等容错（列已存在不视为失败）。
- 迁移 ALTER 遇 `SQLITE_BUSY` / `database is locked` 时重试。
- 不引入 `PRAGMA user_version`（YAGNI），但两边迁移代码必须等价。

**TS upsert 语义（Codex 重要⑥）**：TS 本轮不填这 3 列。`src/db/schema.ts` 的 `upsertSession` 的 `ON CONFLICT DO UPDATE` **不得设置** 这 3 列 —— 否则 TS 重扫会把 Rust 已填的分类清空成 NULL。新列由 Rust 扫描独占写入。UI 聚合把 `NULL`/空 category 归入 `other`，避免 TS 扫出的行从圆环消失。

---

## 8. 文件改动清单

| 文件 | 改动 | 规模 |
|---|---|---|
| `src-tauri/src/lib.rs` | 主体（§8.1，7 处） | 大 |
| `src-tauri/embedded/index.html` | detail 面板加圆环 + 视觉验收（§9） | 中 |
| `src/db/schema.ts` | `sessions` 加 3 列（仅 schema 一致，upsert 不填、不清） | 小 |
| `src-tauri/src/lib.rs` 内 `#[cfg(test)] mod tests` | `classify` 单测（§10） | 中 |

### 8.1 `lib.rs` 的 7 处具体改动

1. **`SessionData` 加 3 字段**（L78）：
   ```rust
   tool_calls: HashMap<String, u64>,
   git_branch: Option<String>,
   activity_category: String,
   ```
2. **`ensure_db_exists`**（L177–290）：在现有 `date` 列迁移之后，追加 3 个 `ALTER TABLE sessions ADD COLUMN` + 各自 `pragma_table_info` 存在性检查（事务内、duplicate 幂等）。
3. **`scan_claude_sessions`**（L327–483）解析循环里：
   - 统计 `message.content[]` 中 `type == "tool_use"` 的 `name` → 写入 **当前 day bucket** 的 `tool_calls`（Codex 重要①：按 `(session_id, date)` bucket 独立累计，**不跨天共享**，修 TS 侧既有的共享 bug）。
   - **额外读 `message.usage.server_tool_use.web_search_requests / web_fetch_requests`**（Codex 阻断③），分别累加进 `tool_calls["WebSearch"]` / `tool_calls["WebFetch"]`。
   - 取首个非空 `data["gitBranch"]` 填 `git_branch`。
   - `parse_codex_rollout`（L483）同样补 `function_call`/`tool_call` 计数（照搬 TS `parsers/codex.ts` L99；codex 无 gitBranch、无 server_tool_use）。
4. **新增 `classify(&tool_calls, git_branch: Option<&str>, source: &str) -> &'static str`**：§5 规则链 + §4 类别表（本模块常量）。`git_branch` 入参前先过规范化函数。
5. **`upsert_session`**（L1049）：`INSERT` 列表 + `VALUES` + `ON CONFLICT DO UPDATE SET` 加 `activity_category` / `tool_calls` / `git_branch` 三列。**`tool_calls` 列按 `source` 决定**（复审新发现②）：opencode/zcode 传 `NULL`，claude/codex 传 `serde_json::to_string(&map)`（空 map 自然成 `'{}'`）。`full_scan`（L1118）在 upsert 前先 `session.activity_category = classify(&session.tool_calls, session.git_branch.as_deref(), &session.source)`。
6. **`by_category` 下沉进 `DailySummary`**（Codex 阻断②）：`DailySummary`（L31）加 `by_category: Vec<CategoryBreakdown>` 字段；`today/week/month/all_time` 四个 `DailySummary` 各自带一份。`get_data`（L1239）对四个周期各调一次 `get_cost_by_category(conn, start, end)` 填入。新增：
   ```sql
   SELECT COALESCE(NULLIF(activity_category,''),'other') AS key,
          COUNT(*) AS sessions,
          SUM(cost)  AS cost,
          SUM(input_tokens + output_tokens + cache_read_tokens) AS tokens
   FROM sessions
   WHERE date >= ? AND date <= ?
   GROUP BY COALESCE(NULLIF(activity_category,''),'other')
   ORDER BY cost DESC
   ```
   > **关键（复审新发现①）**：用 `COALESCE(NULLIF(...),'other')` 把 NULL/空归入 `other`，**不过滤** —— 这样 TS 扫出的 NULL 行也参与圆环（并入 other），不会从 donut 消失。`CategoryBreakdown.key` 用查询出的归一化 key。
   `get_top_models`（L1194，仍用 `date(start_time)`）**顺手统一到 `date` 列**（Codex 建议②），让同一周期四个模块口径一致。
7. **`RecentSession` 加字段**（Codex 重要⑦）：`RecentSession`（L40）加 `activity_category: Option<String>`；`get_data` 的 `SELECT`（L1260）加 `activity_category` 列并映射。前端 session 列表每行显示类别 emoji（可解释性的最小落点）。

### 8.2 `CategoryBreakdown`（Codex 建议①）

```rust
#[derive(Debug, Serialize, Deserialize)]
struct CategoryBreakdown {
    key: String,         // "feature" / "bugfix" / ...
    cost: f64,
    tokens: u64,
    sessions: u64,
    // 不含 pct —— 前端用 cost / sum(cost) 算（KISS）
}
```
字段已为单词，无需 `rename`；`DailySummary.by_category` 字段名加 `#[serde(rename = "byCategory")]`，与现有 `tokenUsage`/`topModels` 的 camelCase 约定一致（L36/38）。前端读 `d.today.byCategory`。

---

## 9. bar 前端（`src-tauri/embedded/index.html`）

在统计卡（`.grid`）下方、Top Models 上方插「消费类型」卡：
- **SVG donut**：`<circle>` 的 `stroke-dasharray/offset` 画弧段，不引库。中心显示该周期总 cost。配色取自类别表（§4）。
- **各类条形**：复用 `.mr` 样式，`emoji + 中文名 + 金额 + 占比条`。占比前端算（`cost/sum(cost)`）。
- **410px 视觉验收（Codex 建议④）**：固定 donut 直径与 legend 行高；超过 5 类时只显 Top 5 + 「⚪其他(合并剩余)」；覆盖空态（无分类数据时显示提示）、1 类、9 类、长金额（`$1234.56`）、低成本（`$0.0001`）四种情形。

前端切 Today/7D/30D/All 时，读对应 `D.today.byCategory / D.week.byCategory / D.month.byCategory / D.allTime.byCategory`，随现有 `render()` 刷新。

---

## 10. 测试策略

`classify` 纯函数单测，覆盖 §14 的 Codex 测试用例表（每类的正例 + 边界/反例）。这是阈值日后校准的回归保护。

| 规则 | 正例 | 边界/反例（期望） |
|---|---|---|
| branch bugfix | `{Write:10}`, `origin/Fix/login` → bugfix | `documentation/x` 不命中 bugfix |
| branch docs | `{Write:10}`, `docs/readme` → docs | — |
| branch refactor | `{Edit:10}`, `refs/heads/refactor/api` → refactor | `chore/refactor-api` 不命中 |
| branch feature | `{Bash:10}`, `feature/donut` → feature | `feat-category`（无斜杠，首段=feat-category 不在集合）→ 不命中 |
| !has_tool_detail | source=`opencode`,`{}` → other | source=`claude-code`,`{}` → research |
| total==0 | source=`claude-code`,`{}` → research | source=`zcode`,`{}` → other |
| agent>0.40 | `{Agent:5,Read:5}` → agent | `{Agent:4,Read:6}`(0.40) 严格> 不命中 → 落后续 |
| web>0.40 | `server_tool_use` 5 次 + Read 5 → research | 严格> 边界 |
| Bash>0.50 | `{Bash:6,Read:4}` → debug | `{Bash:5,Read:5}`(0.50) 不命中 |
| Read/Grep/Glob>0.60 且写≤0.20 | `{Read:7,Grep:1,Write:1}` → explore | `{Read:7,Write:3}`(写 0.30>0.20) 不命中 explore |
| Edit>0.40 且 Edit>Write | `{Edit:6,Write:4}` → bugfix | `{Edit:5,Write:5}`(Edit 不>Write) 不命中 |
| Write>0.30 | `{Write:4,Read:6}` → feature | `{Write:3,Read:7}`(0.30) 不命中 |
| 兜底 | `{TodoWrite:2,Skill:2}` → other | 若未来把 AskUserQuestion 等算 agent，需在规则表显式列名 |

集成：构造 fixture `.jsonl`（含 `server_tool_use` 字段）→ 跑 `scan_claude_sessions` → 验证 `activity_category` 写入正确、跨天 bucket 各自独立。

---

## 11. 已知限制（v1 接受，本轮不解决）

1. **opencode / zcode 永远 `other`** —— 外部 SQLite 聚合无工具细节（`parsers/opencode.ts:126`、`parsers/zcode.ts:135` 均 `{}`）。由 `source` 入参保证归 other，非 bug。
2. **无 gitBranch 时 refactor/docs 并入 feature** —— 工具模式相近，只有对应分支才触发。要更准需提取文件扩展名（§3 已排除）。
3. **codex parser 的 tool_call name 字段路径** —— TS `parsers/codex.ts:99` 提取 `function_call`/`tool_call`，Rust 照搬时以真实 rollout JSONL 为准（实现时校验）。
4. **加重 §1.4 两套实现债** —— §6.2 已诚实标注为短期押注；消解靠 §12。
5. **阈值是经验值** —— v1 先跑，靠 §10 单测 + 真实数据校准。

---

## 12. 演进路径

1. **本轮**：Rust 侧实现分类，bar detail 面板亮起圆环。TS 侧仅同步 schema 列（不填值）。
2. **架构统一（`PRODUCT_PLAN.md` §1.4）**：bar 改调 TS CLI/HTTP，Rust 不再自己解析。届时 Rust 的 `scan_*` / `classify` / `upsert` 随之删除，分类逻辑只剩 TS 一份 —— 技术债反向消除。
3. **TS 侧分类**：架构统一后补 Web/CLI 展示；那时把类别表抽成 TS/Rust 共享 `categories.json`。

---

## 13. 与 PRODUCT_PLAN 的关系

| PRODUCT_PLAN 条目 | 本设计关系 |
|---|---|
| §2 按维度统计 | 延伸：新增「活动类型」维度，与现有维度正交 |
| §1.4 两套解析实现 | 本轮**短期加重**（Rust 补全），长期随统一消除（§12） |
| Phase 2 维度排行 | 圆环 + 占比条即「活动类型排行」，可并入 Phase 2 |
| §0 本地优先 | 严格遵循：纯规则、零云调用、零 Key |

---

## 14. Codex 审核处置对照（v2 修订依据）

| # | 严重度 | Codex 发现 | v2 处置 |
|---|---|---|---|
| 1 | 阻断 | `classify` 缺 source，opencode/zcode 误分 research | ✅ §5 加 `source` 入参 + `!has_tool_detail → other` 前置 |
| 2 | 阻断 | `by_category` 顶层无法适配 tab 切换 | ✅ §8.1-6 下沉进每个 `DailySummary` |
| 3 | 阻断 | WebSearch 信号在 `usage.server_tool_use`，Rust 漏读 | ✅ §4/§8.1-3 补采集并映射成工具计数 |
| 4 | 阻断 | Rust 补全违反 PRODUCT_PLAN §1.4 | ✅ §6.2 重新裁决，标注为短期押注（只挂 bar 用户决定性） |
| 5 | 重要 | 跨天 toolCalls 共享 | ✅ §8.1-3 按 day bucket 独立累计 |
| 6 | 重要 | gitBranch 规范化 | ✅ §5 ① trim/lowercase/去前缀 |
| 7 | 重要 | Write 在 Edit 前误分 bugfix | ✅ §5 ② Edit 提前 + `Edit>Write` |
| 8 | 重要 | explore「写低」无数值 | ✅ §5 ② `≤0.20` |
| 9 | 重要 | 双迁移并发竞态 | ✅ §7 事务 + 幂等 + 两边列定义一致 |
| 10 | 重要 | TS 加列不填产生混合数据 | ✅ §7 TS upsert 不设新列 + UI 归 other |
| 11 | 重要 | RecentSession 缺分类字段 | ✅ §8.1-7 加 `activity_category` |
| 12 | 建议 | CategoryBreakdown derive/serde | ✅ §8.2 |
| 13 | 建议 | top_models 口径不一致 | ✅ §8.1-6 统一到 `date` |
| 14 | 建议 | tool_calls NULL/`{}` 语义 | ✅ §7 |
| 15 | 建议 | 410px 视觉验收 | ✅ §9 |

被忽略方案：Codex 的「TS 填分类、Rust 只查」折中 —— §6.2 已评估并否决（只挂 bar 用户无分类数据）。
