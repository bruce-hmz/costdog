# CostDog 仓库交接文档（供 Review Agent）

> 生成时间：2026-09-29。作者：ZCode agent（本会话）。文档目的：向接手 review 的 agent 完整交代现状、架构决策、已知问题与验证方法。

## 1. 项目一句话

CostDog（v0.4.0，Tauri 2 + Rust + 单文件 HTML 前端）：监控本机 AI 编程客户端（ZCode / Codex(ChatGPT 桌面端) / Claude Code / OpenCode）的 token 用量、花费与缓存，以**桌面宠物（像素柴犬，ChatGPT 生成精灵图）**形态常驻展示，辅以菜单栏下拉洞察面板。仓库：`/Users/bruce/workspace/claude-projects/costdog`，remote `github.com/bruce-hmz/costdog`（**149 个本地提交均未 push**）。

## 2. 代码地图（r~7600 行核心）

```
src-tauri/
  src/lib.rs (4858 行)      — 命令层：扫描/analytics/预算/洞察/会话指标/速率/本地数据服务(9401)
                             /窗口管理(宠物全屏窗+panel面板)/托盘/嵌入命令。setup 顺序敏感！
  src/dock.rs (892 行)      — CGWindowList FFI 客户端检测/前台守卫/光标守卫(NSEvent)
                             /属性重申看门狗/App Nap 禁用/SUMMON 计数。宠物窗相关全在这。
  src/analytics.rs budget.rs cost_ledger.rs source_status.rs  — 分析/预算/定价/数据源状态
  embedded/
    pet.html (145 行)       — 桌面宠物页面（独立小文件！见 §4 铁律）
    index.html (~1200 行)   — 主胶囊(已退役隐藏)+panel 下拉洞察面板（#panel 模式）
    walk-sheet.png 等       — 8 帧走路雪碧图+三宠物(狗/猫/兔)跑睡帧（ChatGPT 生成+PIL 处理）
  scripts/costdog-inject.mjs — CDP 注入器（实验性，未启用）
src/parsers/ src/db/         — TS 版解析器（早期 CLI 遗产，Rust 已接管扫描）
CLAUDE.md                    — **第 5-69 节为本会话全部迭代记录，review 必读**
```

## 3. 当前运行形态（部署于 /Applications/CostDog.app）

1. **桌面宠物**（主形态）：全屏透明静窗（**窗口永不被程序移动**）+ 狗在窗内 CSS 移动沿屏幕底边（Dock 上方 ~110px）散步；8 帧走路循环=CSS 雪碧图 steps 动画；头顶两行铭牌（会话名·CTX / ▲速率·$今日）；停顿张望/转向（只翻狗图不翻铭牌）；agent 静默 60s 趴睡（72px+呼吸动画+Zzz）；⇄ 切换三只宠物；点击铭牌循环/锁定会话。
2. **托盘**：左键弹洞察面板；右键菜单含「🐕 召唤小狗」（隐身自救：SUMMON 计数→前端轮询→狗瞬移屏幕中央+强制重绘）。
3. **本地数据服务** `127.0.0.1:9401`：/stats.json（实时速率/CTX/命中）、/pet.json（**狗矩形坐标+summon 计数——自主验收基建**）、/pet-*.png 素材。CORS 全开（仅回环）。
4. **退役代码**（保留未删）：嵌入宿主胶囊全套（dock.rs 几何分支）、CDP 注入器、cap-chatgpt 窗口。

## 4. macOS 透明窗「铁律」（血泪教训，review 重点核对）

本轮 ~50 个提交在与 macOS window server 博斗中沉淀，违反任何一条都会复现"隐身/影子/铭牌消失"：

1. **程序化 set_position 移动非激活窗 → 移动帧不被合成**（移动即隐身）→ 一切移动必须在窗内做（CSS）。
2. **换 img.src 的高频帧动画 → 重绘风暴挤出兄弟层**（铭牌消失）→ 用 CSS 雪碧图 background-position steps 动画。
3. **页面 absolute+inset 全铺布局 → 透明窗合成死区** → 用 fixed 底部横条容器。
4. **App Nap 节流"永不激活窗"的 WKWebView JS 定时器 → 恍惚残影** → setup 时 NSProcessInfo beginActivity(LatencyCritical) 禁用（dock.rs::disable_app_nap）。
5. **早期设置的 styleMask 会被 Tauri 竞态清除 → 窗口降级被剔除** → reinforce 看门狗每 3s **主线程**重申（后台线程调 AppKit 必崩）。
6. **Tauri WebviewUrl::App 不能拼 #hash**（破坏加载）；多窗口模式判定用 metadata.currentWindow.label。
7. 透明窗三件套：builder `.transparent(true)` + conf `macOSPrivateApi:true` + Cargo feature `macos-private-api`。
8. 主应用大页面与透明窗共用会渲染异常 → 透明窗一律独立 HTML（pet.html）。
9. **自主验收闭环**（用户要求"修完自己看"）：/pet.json 实时坐标 → screencapture 按坐标裁剪 → **Read 工具亲眼看图**（AX/像素计数/窗口级截图都骗过人，只有看整屏合成截图可信）。

## 5. 数据语义（易错点）

- 速率：`zcode_live_output_rate()` = 最近一条真实生成（output≥50 过滤标题类小请求）的 TPS，×60 进 tokens_per_min 字段（曾漏乘 60 导致显示恒 0-1）；生成中实时、完成保 45s、后归零。
- 缓存命中率分母**按源分支**：codex 的 input_tokens 存的是"未缓存输入"（计价正确），分母须 +cache_read；zcode/claude 直除（曾因此爆出 4752%）。
- 会话指标：ZCode 读其 sqlite `model_usage`（含 duration_ms/ttft）；Codex 解析 rollout JSONL（user message=turn 边界）。codex 会话列表=文件名 id+首行 cwd（scripts 内 codex_recent_sessions）。
- 宿主名映射：Codex 桌面端=ChatGPT.app（窗口 owner "ChatGPT"）；数据源 host 'chatgpt'→source 'codex'。

## 6. 已知未解决问题（review 优先级排序）

| # | 问题 | 现状/线索 |
|---|---|---|
| 1 | **间歇隐身未 100% 根除** | 修掉 8 个诱因后实测 2 分钟 10 采样全可见，但用户仍偶发"不见"（同刻实拍可见——疑与前台应用/系统状态相关）。缓解=召唤开关。建议 review：窗口服务日志抓剔除事件，或换 NSPanel 子类方案 |
| 2 | 猫/兔走路图 | **已解决**：三只宠物都有 8 帧走路雪碧图（猫 856×72 pitch 107、兔 704×72 pitch 88，狗 694×72 pitch 86.75），流程脚本化在 `src-tauri/scripts/`（见 §9.7）。遗留：新生成的猫/兔行走帧是**像素风**（边缘对比度 14.7~21），与它们自己的睡姿帧（2.5D 渲染，3.8~4.4）风格不同 —— 狗本来就是"走路像素风/睡觉渲染风"，现在三只统一成这个约定了；想要走路也保持渲染质感需重新生图（措辞要更强） |
| 3 | 宠物拖拽 | **已支持**（见 §9.8）：窗内拖动（不移窗），Pointer Events + 4px 阈值区分点击/拖动，拖动期间光标守卫强制保持可交互 + 10s 超时自愈。手感调优点待用户实测反馈 |
| 4 | 会话切换芯片（⇄）点击区小 | 自动化点不中，仅人工可点 |
| 5 | CDP 注入路线未完成 | costdog-inject.mjs 已写好（Dream Skin 模式注入宿主 DOM），embed_zcode/embed_codex 托盘命令会重启对应客户端带调试端口；因用户转向宠物形态而搁置 |
| 6 | 149 提交未 push | remote 为 bruce-hmz/costdog；push 前建议 squash 整理（大量"修了又修"的隐身系列） |

## 7. 构建/验证方法

```bash
cd ~/workspace/claude-projects/costdog
npx -y "@tauri-apps/cli@2" build --config '{"bundle":{"createUpdaterArtifacts":false,"targets":["app"]}}'
cp -R src-tauri/target/release/bundle/macos/CostDog.app /Applications/ && open /Applications/CostDog.app
# 验证（自主闭环）：
curl -s 127.0.0.1:9401/pet.json        # 狗实时坐标 {x,y,w,h,summon}
curl -s 127.0.0.1:9401/stats.json      # 速率/CTX/命中
screencapture -x -R<x-25>,<y-40>,200,250 /tmp/dog.png  # 按 pet.json 坐标裁剪
# 然后 Read /tmp/dog.png 亲眼看（勿信 AX/窗口级截图）
```
注意：npm 包名须加引号（`"@tauri-apps/cli@2"`，曾因 shell 展开报 EINVALIDPACKAGENAME 假失败）；测试跑 `cargo build` + 前端 `node --check`。

## 8. Review 建议 checklist

1. lib.rs setup 顺序（App Nap 禁用→数据服务→宠物窗→守卫线程）与窗口生命周期是否有竞态残留。
2. dock.rs 的 objc2 手写 FFI（NSPoint Encode、styleMask/level/collectionBehavior、NSProcessInfo）类型正确性——曾因后台线程调用崩溃。
3. pet.html：确认无换 src 动画、无 inset 全铺、setDog 的 clamp（SW-160）。
4. 9401 服务仅绑 127.0.0.1 且 CORS *（本机工具可接受，但 review 确认无监听 0.0.0.0）。
5. CLAUDE.md §5-69 与 git log 对照，确认文档声称的每个"已修复"都有对应提交与验证记录。

## 9. 2026-09-29 晚 Review 修复记录（DSH review agent）

完整 review 见 `HANDOFF-REVIEW.md`。本轮修掉 4 个 P0（均在部署后的实例上验证过）：

1. **睡姿精灵永不显示 → 静默 60s 后狗彻底消失**（`embedded/pet.html`）：`<img id="dog-sleep">` 的 `hidden` 属性从无代码移除，sleep 态下 `#dog-run` 被 `body[data-state="sleep"]` 隐藏、`#dog-sleep` 被 `[hidden]` 隐藏 → 两个精灵都不画。现可见性只由 `data-state` 决定（删 `hidden` 与 `img.sprite[hidden]` 规则，改为 `body:not([data-state="sleep"]) #dog-sleep{display:none}`）。逐版核对：自 9f2cc99（09-26）以来没有任何一版能显示睡姿。
2. **走路雪碧图被声明成 8 倍大 → 走路是一条 88×19 涂抹**（`embedded/pet.html`）：素材真值 694×218（8 帧 ×86.75px），而 CSS 写 `background-size:5552px 72px`(=694×8) / `to{background-position:-5552px}`。改为 `694px 218px` / `-694px`，元素宽 88→86px。
3. **App Nap 实际没被禁用**（`src/dock.rs`）：`beginActivityWithOptions:reason:` 返回 +0 autoreleased token，裸指针一丢活动即自动结束（NSProcessInfo.h 明示），且位掩码 `(0xFF<<20)|(1<<20)|(1<<0)` 没设上 `NSActivityLatencyCritical`(bit 32-39) 与 `IdleDisplaySleepDisabled`(bit 40)。现为进程级永久持有（`APP_NAP_HOLD`）+ 正确掩码 `0x00FFFFFF | 0xFF<<32 | 1<<40`。**验证：`pmset -g assertions` 出现两条 `CostDog pet animation` 断言（修复前 0 条）。**
4. **托盘「召唤小狗」是死的**（CSP）：`pet.html` 内 `fetch('http://127.0.0.1:9401/pet.json')` 被 app CSP 的 `connect-src 'self' ipc: http://ipc.localhost` 拒掉（netstat 实测该 fetch 从未发生）。新增 IPC 命令 `get_summon_count` 取代（app 命令不受 ACL 限制，宠物页本来就走 IPC）。

本轮验证手段（不依赖屏幕录制权限）：`cargo test` 38/38；内联脚本 `node --check`；headless Chrome 读 computed style + canvas 复现 CSS 背景 → sleep 态 `#dog-sleep`=block / `#dog-run`=none，帧墨迹 84×72（新）vs 88×20（旧）；部署后 `/pet.json`、`/stats.json`、`lsof`（仅 127.0.0.1）、`pmset` 断言、unified log（宠物页 500ms IPC 节拍存活、无新增崩溃）。

**本轮（09-30）已修**：① zcode 速率纳入 `status='running'`，并把 `duration_ms` 为 0 的进行中行按"已流逝"折算（否则 `now<end` 分支对已完成行恒为假）；② 命中率分母改为**按源语义声明**（`input_includes_cache()`：只有 zcode 的 input 含缓存读），claude-code / opencode / dsh 一并修正（此前 claude-code 1432%、opencode 1204%）；③ 面板窗口 capability 补上（`windows: ["main","panel"]`，事件订阅与 app 版本查询不再被 ACL 拒）。

**仍未修**（优先级见 REVIEW 报告）：光标守卫重复 spawn 两份；`NSPoint` 编码写成 `{NSPoint=dd}`（真机 `{CGPoint=dd}`，debug 构建会让守卫线程 panic）；`full_scan()` 在 setup 里同步跑、阻塞首帧；~9.0MB 无引用素材；`npm run test:ts` 目前是红的（index.html 947 处 CJK vs 断言 null，且没有测试覆盖 pet.html）；TS 侧（`src/parsers`）尚未接入 DSH。

5. **铭牌离宠物 70px**（`embedded/pet.html`）：全文屏改写时留下的布局残留——铭牌 `top:-2px` 钉在 170px 容器顶部，而宠物被 `justify-content:flex-end` 压在底部。实测空隙 70.3px。改为贴底锚定：`.plate{bottom:80px}`（宠物盒高 72 + 8px 头顶间隙），`#switcher`/`#zzz` 移到 `bottom:114px`（铭牌上方一行，原来 `top:16px` 是**压在铭牌上**的）。Chrome 实测三只宠物 × 跑/睡/停 六种组合空隙均为 8.0px。
6. **⇄ 芯片做成真切换**（`embedded/pet.html`）：原点击处理只有 `stopPropagation()`（三宠物切换逻辑在 9f2cc99 回滚时丢了，猫/兔素材完全不可达）。现为每只宠物一对元素（跑/睡共 6 个），可见性 = `body[data-pet] × body[data-state]` 的 CSS 矩阵，**全程不换 `img.src`**（铁律 2）；点击循环 柴犬→橘猫→灰兔 并写入 `localStorage['costdog.pet']`，标题动态提示"当前：X 点击换成 Y"。顺带修掉两个 review 里点出的老 bug：①转向改用独立 `scale` 属性（原行内 `scaleX(-1)` 被 `stepbob` 的 `transform` 动画覆盖 → 走动时宠物从不回头）；②白描边选择器 `img.sprite` → `.sprite`（狗的跑图是 div，原选择器把它漏掉，走路态没有描边）。
7. **猫/兔也补上 8 帧走路雪碧图 + 生图链路脚本化**：`embedded/cat-walk-sheet.png`(856×72)、`embedded/rabbit-walk-sheet.png`(704×72)，三只跑图元素统一成 `div.sheet` + `background-position steps(8)`（与狗同机制，零重排）。新增两个可复用脚本：
   - `src-tauri/scripts/chatgpt-gen-image.mjs`：CDP 驱动隔离 profile 里已登录的 ChatGPT 生图（新会话→上传参考图→输入提示词→回车→等新图→canvas dataURL 落盘）。实测成品稳定 2172×724 / 8 帧横排，约 40~70s。文件头写了启动 Chrome 的完整命令与踩坑（composer 不是 `#prompt-textarea`；结果有两份 blob；参考图缩略图也会被 `img` 命中，必须按 `naturalWidth` 过滤）。
   - `src-tauri/scripts/make-walk-sheet.py`：横排原图 → 去白底（**从四边泛洪**，白毛角色不会被抠掉）→ 按模型自带等分列切帧（避免逐帧 bbox 居中导致身体左右抖）→ 缩放到 72 高 → 等距拼接，并打印逐帧墨迹/帧间差异与可直接抄用的 CSS 参数。
   - 提示词模板（狗/猫/兔同一套，只换物种）：「生成一张图片：<物种>侧面走路循环精灵图（sprite sheet），横向排列8帧，从左到右构成一个完整的步行周期（迈前腿→四腿交替→蹬地→回收）。严格保持参考图里这只<物种>的角色形象、配色、渲染风格与大小比例完全一致，纯白背景，帧间等距，每帧姿态基线一致，无文字、无边框、无地面阴影。」
   - 验证：headless Chrome 用 Web Animations API 把动画拨到 t=0/500/999ms，三只的背景位移分别精确落在 4×/7× pitch（狗 -347/-607.25、猫 -428/-749、兔 -352/-616）→ 8 帧逐步推进无误；三只 × idle/run/sleep 九种组合都只显示一个元素、与铭牌空隙均为 8.0px。
8. **拖动支持**（`embedded/pet.html` + `dock.rs` + `lib.rs`）：铁律 1 禁止程序化移窗，所以拖动**只改宠物在窗内的 left/bottom**（窗口是全屏的，视觉上等价于拖着它在桌面上走）。要点：
   - Pointer Events + **4px 阈值**区分点击与拖动（阈值内照旧触发铭牌切会话 / ⇄ 切宠物；拖动过就不触发，避免"拖完顺手切了宠物"）；`-webkit-user-drag:none` 挡掉图片原生拖拽；`setPointerCapture` 保证移出元素后仍收得到 move。
   - 拖动期间 `body.dragging` 把 `.critter` 的 `left 1.2s` 缓动关掉（否则宠物追不上光标）、光标切 `grabbing`、漫游状态机冻结（`stateUntil=now+1e9`）；松手后停 3s 再从新位置继续散步，**纵向位置会成为新的散步高度**。
   - 新增 IPC `set_pet_dragging(bool)`：光标守卫在拖动期间**强制保持窗口可交互**。这是老问题（§6.3）的根因——快速拖动时光标会瞬时离开宠物矩形，守卫切回穿透 → pointermove 断流 → 拖动卡在中途。带 **10s 超时自愈**（`PET_DRAG_AT` 存时间戳而非 bool），万一 pointerup 丢了也不会让全屏窗一直吃掉用户点击。
   - 守卫轮询 **80ms → 40ms**：这个周期就是"光标移入宠物→解除穿透"的延迟，直接决定"按下能不能抓住"。
   - 验证：headless Chrome 合成 PointerEvent → 位移精确（抓点 +200/−60 得到 Δ=(200,−60)）、拖动中 `transition=0s`/`cursor=grabbing`/`body.dragging=true`、松手恢复 1.2s；拖到屏幕外夹取到 `left=innerWidth−160` / `top=30`（避开菜单栏）；拖后立刻点 ⇄ 不切换、2px 抖动不触发拖动；`cargo test` 38/38。

9. **接入 DeepSeek Harness（DSH）数据源**（`src-tauri/src/dsh.rs` 新增 + `lib.rs` 五处接线）。用户日常主力已切到 DSH，CostDog 必须能算它的账。
   - **数据源**：`~/.dsh/sessions/<project-slug>/<session-id>/session.v4.jsonl.zstd`（zstd 压缩 JSONL，每写一条事件追加一帧）+ 辅助的 `~/.dsh/storages/session_projcache/sessions/<id>.json`（几 KB，DSH 自己的会话缓存，给模型名/CTX/会话计时用，读它不用解压）。
   - **语义（逐条校验过）**：① `assistant/message.data.usage.inputTokens` 是**未缓存输入**——`input + cacheRead == totalTokens - output` 恒成立，与 Codex 同族，所以命中率分母必须补回 cacheRead；② `totalTokens` 是**该步**合计（含 cacheRead），不是会话累计，累计要自己按步求和；③ 每一步都把整个上下文重新计费一次，逐条求和即真实计费口径；④ `provider/model` 在 `request/header.config` 里。
   - **跨天必须分桶**：实测一个会话 22:14 → 次日 09:12，整会话塞给一天会让"今日花费"虚高。按消息时间戳取**本地日期**分桶，与其它源同口径。
   - **坑（本轮最大）**：DSH 的会话文件是**多帧 zstd**（单文件实测 1032 帧），而 `ruzstd::decoding::StreamingDecoder` **只解第一帧**——真实文件第一帧只有 301 字节，正好是那条 `{"type":"session"}`，于是每个会话都被判成"没有用量"而静默跳过（`malformed_lines=4`、`scan_files` 里 0 条指纹、1ms 扫完）。改为 `FrameDecoder` 按帧循环：`init → decode_blocks(All) → collect_to_writer`，`init` 失败即结束。已加"两帧 zstd"单测（用 ruzstd 自带压缩器现造）锁死。
   - **实时链路**：`dsh_live()` 解压**最近改动**的会话文件，取最后一条消息的"输出/生成耗时"算 TPS（首块 stream 时间 → 消息时间），45s HOLD 语义与 zcode 一致；CTX = 最后一条消息的 `input+cacheRead+cacheWrite`（实测等于 DSH 自己的 `contextPressure.pressureTokens`：443731 vs 443791）。解压有成本，所以 **3s 缓存**，且命令整体挪到线程池（见 §10）。
   - **定价必须 provider 感知（否则高估 3.8 倍）**：`deepseek-v4.1-flash` 在 OpenRouter 缓存里是 0.3/1.2，而缓存读会被 `resolved_openrouter_price` 的 Anthropic 式启发（input×0.1=0.03）放大 10 倍；用户实际走 **opencode-go**，DSH 自带模型表写的是 **0.15/0.6/0.003/0**。新增 `provider_price()` + `sessions.provider` 列（DSH 填、其它源 NULL → 历史行为不变），定价顺序 = (provider,model) 表 → OpenRouter → 兜底表。实测该会话：**$3.88 → $1.03**。
   - **前端**：面板客户端卡片显示名（`SRC_LABELS`：dsh→DSH）、诊断徽章 `.bds`；`list_recent_sessions`/`get_session_metrics` 的 dsh 分支读 projcache（不解压）。测试：`cargo test` 47/47（含 8 条 dsh 单测：多帧解压、跨天分桶、空会话丢弃、TPS、projcache 解析与缺省兜底）。

10. **"跑着跑着卡一下"根因与修复**（本轮由用户提问驱动，`sample` 取证）。
    - **取证**：`sample` 抓 4s 栈，主线程 2691 个样本里 **734 个在 `list_recent_sessions` → `sqlite3_step` → `pread`（真磁盘读）**。原因三连：① 该命令是普通 `#[tauri::command]`，Tauri 的同步命令**在主线程执行**（宏源码 `ExecutionContext::Blocking`；`(async)` 才走 `sync_threadpool`），sample 栈里整条链就在 wry 的 IPC 回调内；② 宠物页 `tick()` **每 2s** 调它一次；③ 它对 ZCode 库的 SQL 是 `JOIN session ... GROUP BY s.id ORDER BY MAX(started_at)`，真库 28,946 行上实测 **945ms**（全表扫 + 临时 B 树）。
    - **修复四件**：① SQL 改为"先用 `(started_at)` 索引取最近 300 条请求定位候选会话，再只在候选会话内聚合"（`SCAN ... USING INDEX model_usage_started_model_idx`，冷缓存 ~50ms、热 <5ms，仍返回最近活跃 5 个会话）；② `get_live_stats` / `list_recent_sessions` / `get_session_metrics` / `get_analytics` / `scan` 全部改 `#[tauri::command(async)]`（挪到线程池，主线程再不会被查询堵住）；③ `list_recent_sessions` 加 3s 结果缓存；④ 宠物页分频：速率仍 2s，今日花费 16s，会话名/CTX 10s（或活跃源变化时立即刷）。
    - **顺带**：3s 一次的属性看门狗原来**无条件** `setStyleMask:/setLevel:/setCollectionBehavior:`——`setStyleMask:` 会重建窗口 frame/backing 并触发 WindowServer 往返，等于每 3s 主动卡一下；改为**先读后写**（值已正确就什么都不做，职责仍是"被清掉时补回来"）。另外差分速率的 `MIN_DELTA_SECS` 2.0 → 1.0（宠物 2s + 面板 5s 交错时相邻采样只差 1.x 秒，旧值会把这类差分全丢掉）。
    - 复验：部署后 `sample` 主线程不再出现 `list_recent_sessions`（已在线程池线程），`/stats.json` 三源并存（codex / zcode / dsh，dsh 速率与 CTX 正常）。

11. **清掉剩余 P2 + TS 侧接 DSH + UI 文案统一英文**（本轮，收尾）。
    - **光标守卫只起一份**：`spawn_cursor_guard` 被 `setup` 与 `ensure_topbar_window` 各调一次 → 两条线程各自维护 `pass_through`，每次边界穿越写两遍 `set_ignore_cursor_events`。加 `GUARD_STARTED` 幂等开关并删掉重复调用点。
    - **`NSPoint` → `CGPoint` 编码**：`dock.rs` 手写的 `Encode` 用了 `"NSPoint"`，而方法签名是 `{CGPoint=dd}`。**更正我上一轮 review 的说法**：这里不会 panic——A/B 对照（临时改回 `NSPoint` 跑 debug 构建）日志里 0 条 panic，objc2 的 `msg_send!` 并不校验返回类型编码（`verify` 模块只在测试里用）。改动保留（编码名应与 ABI 一致），但性质是隐患而非崩溃。
    - **首扫移出主线程**：setup 里原来同步 `full_scan()`（要遍历解析 codex 那种 14MB rollout），拖住第一帧。改为扫描线程启动时先扫一次再进 30s 循环，扫完 `emit("refresh-data")`。
    - **TS 侧接入 DSH**：新增 `src/parsers/dsh.ts` + `getDshSessionsDir()`（`DSH_HOME` 可覆盖）+ `SessionSummary.provider` + aggregator 接线。**同一个多帧坑**：Node 的 `zstdDecompressSync`、`zstdDecompress` 回调、`createZstdDecompress` 流式三种入口都只解第一帧（实测 226 字节），所以按帧魔数切开逐帧解，魔数误切时并回下一帧重试（实测 1373 帧全解、0 失败）。定价同样加 `PROVIDER_PRICING` 并让 `calculateCost` 接受 provider。
    - **跨前端对账**：TS 侧 `fullScan` 到临时库与 Rust 侧正式库逐行比对，4 个已定稿会话的 token 与 cost **完全一致**（0.0671/0.0416/0.3714/0.0192），唯一的差异行是仍在增长的活跃会话（TS 那次扫得更晚）。
    - **UI 文案统一英文**：`npm run test:ts` 之前是红的——CJK 断言报 947 个字符。查下去发现注释里 901 个 + **真正的中文 UI 文案 46 处**（退役停靠胶囊的 tooltip/pop 标签，以及菜单栏洞察面板的分区标题与洞察句）。改法两件：① 断言先剥注释再看可见文案（把"UI 单一语言"这条守严，而不是让注释淹没它）；② 把 46 处文案全部译成英文（保留 emoji 前缀，面板 `slice(0,2)` 的图标逻辑依赖它）。**测试现在 14/14 全绿**，并验证过它仍能抓到真实违规（往 `pv-sec` 塞回中文 → 立刻失败）。

12. **"走起来生硬 + 最后还要滑步"**（用户反馈驱动；代码 + 素材两层都有账）。
    - **残余滑动（代码）**：`.critter{transition:left 1.2s linear}`，而状态机每 110ms 就写一次 `left`；停下时腿已经定格（`body:not(.walking)` 把帧动画清掉），但**在途过渡还会滑满 1.2 秒**。改成 `120ms`（≈tick）后，停下最多再走 1.6px。实测残余 0px。
    - **脚打滑（素材 × 参数）**：地面速度必须 ≤ 步幅 ÷ 循环时长。旧参数 `SW*0.004`（1920 屏 ≈70px/s）而狗一个 8 帧循环里脚只前后走 **8.6px**（10% 身宽）→ 上限仅 8.6px/s，差了 **4.9 倍**，所以整段路都在冰上滑。三处修：① 帧循环 1s → 0.6s；② 速度改为**按宠物分别取**（`PET_STRIDE` 表 ÷ 循环 ÷ tick），换素材就改这张表；③ 用绝对像素而不是 `SW` 比例 —— 否则 4K 屏上宠物自己加速到 140px/s，打滑程度随显示器变化。
    - **"生硬"（素材风格）**：旧三张是像素风（边缘锐度 61.9 / 62.0 / 54.1），而睡姿帧是平滑渲染（13.9）；公平基准（参考图缩到 72px 高的锐度）是 47~52。重画后：狗 42.7、兔 46.0、猫 49.5 —— 都比基准更平滑。
    - **重画（3 张，`chatgpt-gen-image.mjs` + `make-walk-sheet.py`）**：步幅 狗 8.6→**19.5**、兔 21.7→**23.1**、猫 21.3→**13.4**（猫这张步幅小了些，速度表跟着降到 22px/s，仍不打滑）。提示词里新增两条硬约束：**版式**（同一行 8 只、间隔 ≥40px、不许粘连/第二行/网格）与**迈步**（触地前爪相对身体后移约 1/3 身长）—— 前一次没写版式，模型画成 5 只还互相粘住，切图直接失败。
    - **`make-walk-sheet.py` 增测步幅**：打印逐帧脚位与 `stride`，并给出 400/600/800ms 三档循环下"脚不打滑的地面速度"，把"素材 → 参数"这条关系钉死在工具里。
    - **生图链路的两个新坑（已写进脚本注释）**：① 合成 Enter **必须带 `text/unmodifiedText:'\r'`**，否则内容可编辑区把它当普通按键吞掉（实测 composer 里留住整段提示词、页面零条消息；这个版本也没有 `data-testid="send-button"` 的发送按钮）；② 发送成功的判据要多路且宽松（composer 清空 / 出现停止按钮 / 页面出现提示词片段，轮询 10s）—— 曾只等 1.2s 就误判失败并开新会话，把**已经发起**的生成中断。
    - 验证：headless Chrome 实测 狗 31.3 / 猫 21.5 / 兔 37.1 px/s（理论 32.5 / 22.3 / 38.4，差 3% 来自首个 tick 不位移）；`walkcycle@600ms`、`stepbob@300ms`、`transition 0.12s` 均在计算样式里核对；部署实例与构建产物二进制 md5 一致。对比图 `walk-old-vs-new.png`（上旧下新、2× 放大）。

13. **睡姿与跑图不同源**（用户："睡眠状态还是之前的宠物形态"）。上一轮只换了跑图（`*-walk-sheet.png`），睡姿仍是更早那批单独生成的 `pet*-sleep.png` —— 两只动物是**不同一次生成**的，所以走起来是新形态、一睡下变回旧形态。
    - 重画三张睡姿（同一套参考图 `pet*-run.png` + 同一套风格约束：平滑 2.5D、不要像素化、闭眼趴卧、纯白背景、视角与参考一致），新增 `scripts/make-sleep-sprite.py`（单张立绘 → 去白底**四边泛洪** + 裁内容 + 按高度缩放；`--height 216` 出 3× 图给 Retina，CSS 仍 `height:72px` 所以不用改尺寸）。
    - 同源度（睡姿 ↔ 新跑图的平均色差，越小越像同一只）：狗 16.9 → **10.7**、兔 24.0 → **8.9**、猫 12.4 → 14.6（都很小，猫略升）。边缘锐度（按 72 高归一）：狗 43.7→33.4、猫 41.1→34.6、兔 38.9→30.3，三只都比跑图更平滑（跑图 42.7/49.5/46.0），和睡姿该有的柔和质感一致。
    - 渲染矩阵复核（headless）：三只 × 睡/跑 六种组合都**只显示一个元素**，尺寸 跑 79~100px / 睡 134~147px，同为 72 高。
    - **生图脚本新坑**：单角色图只有 ~1254px 宽，低于脚本默认 `--minw 1500` 的过滤门槛 → 图明明生成了却报"超时"。单张立绘要 `--minw 900`（该参数本意是滤掉参考图缩略图）。对比图 `sleep-old-vs-new.png`。

14. **⇄ 切换动物"失效"**（用户反馈）。根因是**指针捕获把 click 抢走了**，不是切换逻辑坏了。
    - 机制：`onDragDown`（critter 的 pointerdown）无条件 `critter.setPointerCapture(e.pointerId)`。一旦捕获，后续指针事件的 target 都变成 critter，浏览器把 `click` 派发到"按下目标与抬起目标的最近公共祖先"= critter —— 于是 ⇄ 芯片（和铭牌）的 click 处理器**永远收不到**。这条是拖动功能引入的（比用户上一次成功切换更晚），所以表现为"以前能切，现在切不动"。
    - 为什么我前两次测试没抓到：我用的是 `element.click()` / `dispatchEvent(new MouseEvent('click'))`，这两条都**绕过浏览器的命中测试与捕获重定向**，处理器照样触发 —— 测试通过但线上是坏的。**权威做法是用 CDP `Input.dispatchMouseEvent` 走真实输入管线**（这次的 A/B：修前点击后 `pet`/localStorage 都不变；修后单击三下 dog→cat→rabbit→dog，localStorage 同步）。
    - 修法：① 捕获改到 `onDragMove` 里、越过 4px 阈值确认是真拖动之后再做（普通点击不再被夺走 click）；② `pointermove/up/cancel` 改挂 `window` —— 捕获发生前那几像素位移也要收得到，否则从宠物边缘往外拖会丢事件起不来。
    - 顺带三处硬化（同一条"点了没反应"的排查链）：① 宠物矩形上报 **500ms → 120ms**（换素材后狗走 32px/s，500ms 滞后就是 16px，光标压在狗身上、守卫拿的却是过期矩形 → 窗口保持穿透 → 点击丢失）；② 守卫命中 margin **12 → 20px**；③ 守卫状态变成**可观测**：`/pet.json` 增加 `guard_ticks`（线程活着吗）/`interactive`（窗口当前可点击吗）/`cx,cy`（它看到的光标位置），并且 `set_ignore_cursor_events` 写失败时不再更新本地状态（以前写失败也照样翻 `pass_through`，会让窗口**永久**停在穿透态且再不重试）。
    - 验证：CDP 真实输入 单击⇄ 三次循环正确 + localStorage 同步；拖动仍 Δ=80px；点铭牌不误触拖动。真机侧：`/pet.json` 显示光标移到宠物中心时 `interactive=true`、移开变 `false`（守卫读数与瞬移坐标逐像素一致）。
    - **环境限制**：本机 `CGPreflightPostEventAccess()=被拒`（无辅助功能权限），CGEvent 合成点击会被系统静默丢弃 —— 所以"真机点一下"这条路走不通，验证必须走 headless CDP 输入管线。

15. **走路"像抽搐"**（用户反馈）。先排除素材：三张新雪碧图的逐帧顶/底完全对齐（极差 0px）、纵向重心只差 0.8~2.5px —— 素材没问题。问题是**运动是怎么驱动的**：
    - 位移只在 110ms 的 tick 里一跳 1.6~4.2px（≈9Hz 阶梯），而腿部帧动画是 13Hz 硬切 —— 阶梯位移 + 逐帧跳变的腿叠在一起就是"抽搐"。
    - 我上一轮把 `stepbob` 从 0.5s 提到 0.3s（为了配合 0.6s 的循环保持"每循环两次"），于是变成 **3.3Hz、上下抖 5px** 的高频抖动 —— 这是第二个抽搐源（幅度是我自己加倍的）。
    - 修法：① 位移改为 `requestAnimationFrame` 逐帧积分（亚像素），并改用 `transform: translate3d()` 只走合成层（不触发布局/重绘）；CSS 里 `transition:left` 整条删掉 —— 顺带让"停下还在滑"那类 bug 不可能再出现；② `stepbob` 幅度砍半（2.5/1.5px → 1.2/0.8px），频率不变（四足对角步态本来就是每循环两次）；③ 状态机里留 rAF 兜底：`performance.now()-lastFrameTs>300` 就按 110ms 补一格，防窗口被完全遮挡时 rAF 被节流把宠物冻住。
    - 实测（headless CDP 真实输入管线）：**61fps，每帧位移 0.53~0.55px（理论 32px/s ÷ 60 = 0.53），73 帧里只有 1 帧为 0，帧间隔 16.5~16.8ms**，样式为 `transform: translate3d(519px,0,0)`。回归：真实点击 ⇄ 两次 dog→cat→rabbit（localStorage 同步）、真实拖动 +80px 都正常。
    - 真机复核：`/pet.json` 的 x 以稳定的 2~3px/120ms 推进（≈32px/s），守卫 ticks 正常。

16. **"还是不协调"**（用户反馈）。这次先把素材量化，发现"协调性"有可判定的判据，于是写了工具 + 按判据重画。
    - **新增 `scripts/check-walk-sheet.py`**：对一张（未切的）生图判 5 条 —— ① 摆动相（至少 2 帧"触地脚簇 ≤2"，即真有脚抬起来）② 抬脚高度（≥3 列的墨迹高于底线 6px）③ 步幅 ≥12% 身宽 ④ **背线极差 ≤4px**（起伏要靠腿，不许整体上下弹）⑤ 各帧墨迹面积极差 ≤25%。用法：`check-walk-sheet.py <原图.png>`，退出码 0/1 可直接当生成流水线的验收闸门。
    - **诊断结果（旧三张的生图原稿）**：狗 PASS；**猫 FAIL —— 八帧全是四脚贴地，没有摆动相，等于原地搓腿**；**兔 FAIL —— 背线极差 15px，整只兔在画面里上下弹 20% 身高**。
    - **"不协调"的第二个来源是我自己**：兔那 15px 的身体弹跳正好撞上我挂在所有跑图上的自由运转 `stepbob`（0.3s 正弦、与帧相位无关）→ 两个起伏打架。**已把 stepbob 整条删除**（连带 keyframes）：素材自带起伏（狗 4px / 猫 2px），想要更多起伏应该在出图时让腿去体现，而不是 CSS 里补一个。
    - **重画猫/兔（二版）**：提示词里把步态要求写成硬约束（"至少 2~3 帧能看到某只脚抬离地面 10~20px""各帧身体高度必须一致，起伏只由腿部姿态体现"）。结果：猫 PASS（摆动相 ✓、步幅 16%、背线 4px），兔 PASS（摆动相 ✓、背线 **15px → 1px** ✓）。切图后步幅 猫 12.9px（21.5px/s）、兔 23.4px（39.0px/s），CSS 尺寸与 `PET_STRIDE` 同步更新。
    - **生图脚本又抓到一个假阳性**：发送校验里用 `document.body.innerText` 找提示词，而**输入框自身的文字也在 innerText 里** → 提示词卡在 composer 没发出去，脚本却报"已发送"（随后干等 300s 超时）。改为只在 `[data-message-author-role], article, .markdown` 里找。
    - 验证（headless）：猫 21.0 / 兔 38.1px/s（理论 21.5 / 39.0），狗 0.53~0.55px/帧（≈32px/s）；三只 × 睡/跑 六种组合仍只显示一个元素；当前动画只剩 `*walk`（stepbob 已无残留）。部署二进制与构建产物 md5 一致。

17. **"宠物四周有一个框框" + "走路是狗、睡着变猫"**（用户反馈，两条都查实并修掉）。
    - **框框 = CSS `filter: drop-shadow()` 沿元素盒子画**。`.sprite` 上叠了三层 drop-shadow（§57 为深色背景可见性加的）。截图 A/B 实测：走路精灵是 `div` + `background-image`，浏览器对这种元素做 drop-shadow 时按**元素盒子**算 —— 元素四边可见率 top/bottom/left/right = 0%/67%/24%/21%，盒外还有 24% 的绘制；把滤镜去掉后变成 0%/14%/0%/0%、盒外 **0.0%**。（`<img>` 才会跟着 alpha 走，所以只有走路时有框、睡姿看着正常。）
    - **修法**：把白描边 + 柔和投影**烘焙进素材**（两个脚本都加了 `add_outline()` 与 `--outline`，默认开），CSS 里 `.sprite` 的 `filter` 整条删除。素材尺寸因此 +4/+6（跑图 100×78 / 79×78 / 83×78，background-size 828/660/692×78），睡姿图也统一烘了描边（426/409/445×222）。复测：元素四边 0%/14%/0%/0%、盒外 0.0% ✓。
    - **"睡着的猫"是我抓图抓错**：上一轮 CDP 抓图取的是"页面上最后一张大图"，而当时猫的生成也在同一页面 → 狗的睡姿图实际是猫。**独立视觉判定确认**：用本地能看图的 CLI（`claude -p`）逐张判定 6 个素材 —— `pet-sleep.png=橘猫`（错）、其余 5 张正确；重画后新图判定 `动物=柴犬 / 姿势=趴卧蜷缩` ✓。**从此素材身份可以用视觉模型验收**（配色相似度区分不了柴犬和橘猫：实测狗睡姿离猫参考 5、离狗 11，颜色指标不可靠）。
    - 教训：CDP 抓图必须"生成前记基线、只取新增的那张"，不能图省事取最后一张；跨物种的颜色相近素材必须用视觉判定而不是均值色。

