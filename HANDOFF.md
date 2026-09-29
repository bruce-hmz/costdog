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
| 2 | 猫/兔无 8 帧走路图 | 仅狗有雪碧图；猫/兔是单帧平移。ChatGPT 生图链路已自动化（隔离 Chrome profile ~/.costdog/chrome-profile + CDP 9222）可复用 |
| 3 | 宠物拖拽手感 | 全屏穿透窗内 drag-region 拖拽与光标守卫（解除穿透条件 ±12px）交互未调优；现拖拽基本不可用 |
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
