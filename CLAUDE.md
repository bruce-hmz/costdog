# CostDog 项目进度

## 项目位置
- 代码目录：`D:\codes\costdog`
- Tauri 应用：`D:\codes\costdog\src-tauri`

## 已完成的工作

### 1. 修复下拉展开按钮问题
- 修改了 `src-tauri/src/lib.rs`，使用 `LogicalSize` 替代 `PhysicalSize`
- 更新了 Tauri v2 API（`__TAURI_INTERNALS__`）

### 2. 修改窗口样式
- 去掉了原生标题栏（`decorations: false`）
- 添加了可拖拽的 bar
- 添加了关闭按钮
- 窗口宽度调整为 410px

### 3. 添加小狗动画
- 在 bar 左侧添加了行走的小狗动画（🐕）
- 动画范围：从 40px 到 -10px
- 动画时间：3秒

### 4. 创建独立应用
- 修改了 Rust 代码，直接读取 SQLite 数据库
- 添加了 `rusqlite`、`dirs`、`chrono` 依赖
- 创建了嵌入式前端文件 `src-tauri/embedded/index.html`
- 构建成功，生成了安装程序

### 5. 实现自动扫描和刷新
- Rust 直接扫描 Claude Code 和 Codex 日志
- 每 30 秒自动刷新数据
- 通过 Tauri 事件系统通知前端

## 生成的文件

- `src-tauri/target/release/bundle/nsis/CostDog_0.1.0_x64-setup.exe` (3.1MB)
- `src-tauri/target/release/bundle/msi/CostDog_0.1.0_x64_en-US.msi` (4.5MB)

## 当前问题

### 数据源扫描问题
1. Claude Code 的实际数据格式和假设的不同
2. `~/.claude/sessions/` 目录下的 JSON 文件只包含基本会话信息（pid、sessionId、cwd），没有 token 使用量
3. `~/.claude/history.jsonl` 包含历史记录，但没有 token 使用量
4. `~/.claude/projects/` 目录下可能有更详细的数据

## 下一步

1. **研究 Claude Code 的实际数据格式**
   - 检查 `~/.claude/projects/` 目录结构
   - 找到包含 token 使用量的文件
   - 确定正确的解析方式

2. **添加 OpenCode 和 Trae 数据源**
   - 需要了解这两个工具的日志路径和格式

3. **修复 Rust 代码中的警告**（snake_case 命名）

## 数据库位置

- 数据库路径：`~/.costdog/costdog.sqlite`

## 构建命令

```bash
# 开发模式
npm run tauri:dev

# 构建安装程序
npm run tauri:build
```

### 6. 停靠 ZCode 窗口（macOS，2026-09-16）

- 需求：不悬浮在任意位置，而是把 bar 贴在 ZCode 桌面端窗口的底边并实时跟随
- 实现：新增 `src-tauri/src/dock.rs`
  - macOS 用 `CGWindowListCopyWindowInfo` FFI（手写绑定，无新依赖）找 owner 为 `ZCode` 的 layer-0 主窗口
  - 500ms 轮询：bar 顶边 = ZCode 底边、水平居中；超出屏幕时收进底边内侧
  - `lib.rs` 新增 `app_prefs` KV 表 + `get/set_dock_zcode` 命令，偏好持久化
  - 前端展开面板 Monthly budget 下方新增 "Dock to ZCode" 开关（`embedded/index.html`）
- 验证：System Events 实测停靠与移动跟随均为像素级对齐（顶边==底边、中心重合）
- 注意：Windows/Linux 暂为 no-op（`#[cfg]` 门控），需要时再接各自窗口枚举 API
### 7. 停靠态实时条：当前项目速率 + token 用量（2026-09-16）

- `lib.rs` 新增 `get_live_stats` 命令：按 source 聚合最近 10 分钟有活动的会话，返回当前项目、token 速率（前后快照差分 + EMA 平滑）、窗口内 token 总量、缓存命中率；多桌面端（zcode/codex/claude/opencode）自动各占一行
- 前端 `embedded/index.html` 新增 `#livestrip`：仅在停靠开启且有活跃会话时显示，逐 3s 轮询；高度 = 36 + 行数×24 + (展开?484:0)，`syncHeight()` 统一管理
- 样式全部走现有 CSS 变量（--panel/--border/--dim/--accent），自动跟随皮肤、明暗模式与主题色
- 验证：窗口高度 60（36+1 行）且跟随移动仍像素级贴底
### 8. 实时行主速率 + 点开双组会话指标（2026-09-16）

- 主行改为 dsh 风格：▲ 实时速率(tok/s) 前置加粗强调，行可点击
- 新增 `get_session_metrics(source)`：直读 ZCode 自己的 `model_usage` 表（含 duration_ms / time_to_first_token_ms / tool_call_count），按当前会话聚合出 8 项指标
- 点开显示两组：会话计时（模型用时/工具调用用时/TTFT/TPS）+ Token 用量与缓存（缓存命中/未缓存输入/缓存读取/输出），中英文标签跟随 UI
- 展开高度 +100（POP_HEIGHT），syncHeight 统一管理；非 zcode 源暂只显示 token 组
- 已知近似：工具调用用时 = 会话墙钟时长 − 模型耗时（含空闲间隙），与 dsh 的纯工具执行口径略有差异
### 9. 停靠态 UI 原生化（对标 NotchNook/statusline 生态，2026-09-16）

- 窗口 `transparent: true`；`dock.rs::apply_material` 在停靠开启时运行时应用 `HudWindow` 毛玻璃材质（NSVisualEffectView），关闭时清除；body/.detail 改透明+显式底色配合
- 实时条排版：tabular-nums 等宽数字、分段式竖线分隔符、body.hud 时条体半透明透出材质
- 新增 CTX 段：最近一次请求输入 token（上下文占用代理），zcode 数据源特有
- 闲置行透明度降为 .5（rate≈0 时）
- 交互：点击展开/收起并钉住（pin）；hover 展开仅窗口聚焦时生效——macOS 未聚焦窗口不投递 mouse-moved，WKWebView 跟踪区不激活，这是平台限制而非缺陷
- 市面参考：Claude Code statusline 生态（ccstatusline/ccusage/ccline）、NotchNook 悬浮岛、VS Code statusbar API、OpenCode#8619
### 10. 停靠态改为原生状态栏外观（2026-09-16）

- 反馈：停靠时仍显示 CostDog 皮肤 bar，嵌入感不足
- 改法：`body.docked` 时隐藏 `.bar`（皮肤面），`#livestrip` 升级为主状态栏（as-bar）：32px 主行（速率前置加粗 + 项目 + 缓存/Σ/CTX + ⌄ + 🐕 详情 + × 隐藏），其余活跃源为 22px 次行
- 高度：停靠 = 32 + (行数-1)×24 + 弹层 100 + 详情 484；无活动时显示占位行
- 未停靠（浮窗模式）完全保留原皮肤 bar 外观；🐕 按钮可随时打开完整详情面板
### 11. 多客户端自动检测与跟随（2026-09-16，Antigravity/Gemini 出行为规范）

- `dock.rs`：`KNOWN_CLIENTS` 清单（ZCode/DeepSeek Harness/Claude/Codex/Cursor/Windsurf，后续可配置化）
- `find_dock_target()`：CGWindowList 前到后第一个命中的已知客户端 layer-0 最大窗口
- 防抖：候选目标连续 2 次轮询（≈1s）才切换，未通过期间继续跟随原目标；无已知客户端在前台保持最近位置
- 停靠 y 改为 `zcode_bottom - 1px` 重叠，配合 ::before 微光顶边消除宿主投影缝隙；`set_shadow(false)` 关自身投影
- 已验证：单客户端回归通过；多客户端并发切换待实测
### 12. 惊叹点打磨 + 终审 PASS（2026-09-16）

- 停靠移动 200ms 平滑滑入动画（5 帧插值，dock_animation_ms 落地）
- 主行内嵌 sparkline：最近 24 个速率采样点的迷你走势图（空闲为平线）
- `get_dock_host` 命令 + body[data-host] 主题 profile：zcode 暗蓝（默认）/codex 浅色/claude 暖色，随停靠目标自动切换
- Gemini 终审 PASS，评语：「sparkline 流态波动与紫调 CTX 契合得天衣无缝，犹如 ZCode 原生天生的极客数据守护犬」
- 待办：多客户端并发切换真机实测；正式包 npm run tauri:build；commit/PR 由维护者确认
### 13. 嵌入模式：进宿主窗口内部底边（2026-09-17，用户反馈驱动）

- 用户要求把条嵌进 ZCode 窗口内部"权限/模型选择器之间的区域"，而非贴在窗外
- dock.rs 嵌入几何：y = 宿主底边 − bar高 − 12px，水平居中；展开内容放不下时向宿主顶部收，始终不越出宿主边界
- AX 探测确认 ZCode web 内容可访问但元素过多，精确定位选择器成本高，v1 用固定内嵌偏移
- 品牌锚点开态改为「⌃ 收起」文字按钮（accent 色），详情展开/收起入口更显性
- 验证：嵌入几何精确（y=812 == 880−56−12）；展开/收起自动点击验证受窗口随动影响，以人工复核为准
### 14. 嵌入落位标定 + v2 胶囊样式（2026-09-17，Gemini 双轮评审）

- 嵌入位置从右下角改为用户指定区域：ZCode 底部输入栏「权限/模型选择器之间的空白」，偏移按 Gemini 截图标定（宿主左缘+730px、距底 47px），窗口移动后相对偏移仍成立
- v2 胶囊样式：24px 高、rgba(255,255,255,0.75)+blur(8px) 浅色玻璃、12px 胶囊圆角、边框 rgba(0,0,0,0.06)、微阴影、文字 rgba(0,0,0,0.75)、accent #2563eb/#16a34a/#9333ea
- Gemini 第四轮最终验收 PASS：「无违和悬浮层感，像 ZCode 原生自带的状态指示器」，确认未遮挡任何芯片/按钮/正文
- 注意：偏移量绑定当前 ZCode 布局，客户端大版本更新 UI 后需重新标定（重新截图→Gemini 测量→改 730/47 两个常数即可）
### 15. 嵌入落位鲁棒化：线性空隙模型（2026-09-17，Codex+Gemini 联合咨询）

- 问题：固定偏移(730/47)在窗口缩放后错位
- Codex/Gemini 一致推荐方案 (b) 两点标定线性模型（Electron flexbox 布局空隙随宽度线性伸缩，零权限零开销）；Codex 建议 (d) AX 定期校准作为升级路径（暂缓）
- 实现：空隙左缘=0.3774w+23.1、右缘=1.2742w−752.1（w=1820/1200 两点实测，Gemini 截图测量）；胶囊左坐标=zx+中心−半宽；空隙<426px 时冻结最近有效位置（降级）
- 修复：初版漏加宿主 zx 偏移，缩放实测（1500/1815 两档）发现并修正
- 验证：w=1500→x=720、w=1815→x=980，均与模型预测一致；y 同步精确
- 已知限制：ZCode 非 flexbox 的断点式布局变化需重新标定；窄窗(<1340)降级为冻结；首次点击激活窗口为 macOS 行为
### 16. 胶囊 v4：克隆宿主材质（2026-09-17，用户截图实测纠正）

- 用户对深色胶囊仍不满意（截图实测：ZCode 输入框为深色 #2b2b2b/#444 边框/文字 #d4d4d4/强调橙 #f97316，内容区为浅色——混合主题）
- 根因复盘：v1-v3 在"深/浅玻璃"两极摇摆，从未克隆宿主自身的视觉语言
- v4：背景 rgba(43,43,43,.92) 同输入框、边框 #444（去底边框）、6px 圆角、无阴影、文字 #d4d4d4、速率强调改用 ZCode 自家橙 #f97316
- Gemini 对比评审 PASS（旧版 vs v4），附微调（去底边框/右 padding）已采纳
### 17. 芯片同排精确对齐（2026-09-17，AX 系统级锚定）

- 用户要求：与左「完全访问」/右模型选择器同一水平线、处于两者正中、文字对齐
- 方法升级：放弃图像识别测量（噪音大、两次误导），改用 System Events AX 直接读芯片精确坐标
- 锚定结果：空隙中点 = 0.5w−19.5（左右芯片簇锚定两侧推得）；主行文字中心距底 48px（=芯片文字中心）
- 修正：y 从"总高锚定"改为"主行锚定"——多行/展开时主行始终与芯片对齐，其余向下延展
- 验证：主行中心 830 == 完全访问文字中心 830；中心 x 984 == 空隙中点 984.5
### 18. 嵌入态单行化：只显示宿主客户端数据（2026-09-17）

- 用户反馈：多出一行 codex 信息、超出输入框/工具栏行范围
- 修正：嵌入态主行与展开面板都锚定宿主数据源（body[data-host] 匹配 source，无匹配回退最活跃源）；
  其他工具的数据不再占行——芯片行恒单行 24px，不下坠、不堆叠
- 浮窗（未停靠）模式保留多行多工具显示
### 19. 缩放跟随修复 + 最小化隐藏 + 空隙自适应（2026-09-17）

- 用户反馈：缩小不跟随；最小化后残留桌面遮挡其他应用
- 根因一：越界夹取边界漏加宿主 zx（相对/绝对混用），缩小触发错误夹取
- 根因二（深层）：旧图像测量线性空隙模型与 AX 实测锚点差 ~260px（把输入框边缘
  误判为芯片），两套标定不自洽——几何段全部重建为 AX 标定：
  左簇右缘=493 常数、右簇左缘=w−532、中心=0.5w−19.5（自洽）
- 空隙自适应：胶囊宽度随空隙收窄（300~410px，set_size），空隙<316px 隐藏
- 目标窗口最小化/关闭/切 Space → 胶囊 hide；回到屏幕 → show 并复位
- 实测：1793/1400/1250 三档宽度（居中/收窄359/隐藏）、最小化隐藏与恢复全通过
### 20. 前台守卫：宿主不在前台即隐藏（2026-09-17）

- 用户反馈最小化场景"还是有点问题"——复现发现Cmd+M/Dock恢复均正常，真正场景是：
  胶囊置顶 + 宿主窗口只要可见(哪怕被浏览器盖住不在前台)就继续浮在宿主底部 → 遮挡前台应用
- 修复：frontmost_owner() 检测前台 layer-0 窗口 owner；前台非已知客户端时隐藏胶囊，
  宿主回前台自动重现；CostDog 自身获得焦点时豁免（点击胶囊/面板不消失）
- 实测：ZCode前台显示/切Chrome隐藏/回ZCode重现位置精确/胶囊自身焦点保持
### 21. 会话切换：指标面板可查看任意最近对话（2026-09-17）

- 用户反馈：8 项指标只能看当前活跃对话，无法查看其他对话
- 新增 list_recent_sessions(source)：最近 5 个会话（项目名/最后活跃/token总量/活跃标记）
- get_session_metrics 增加 sessionId 参数（空=跟随活跃会话）
- 展开面板顶部会话芯片行（跨双栏）：● 绿点=10 分钟内活跃，点击切换查看该会话的
  8 项指标，再点取消回到自动跟随；POP_HEIGHT 118→148
- 实测：AX 列出 5 个会话芯片（ZCodeProject●/ai-live-intelligence●/muse-voice-transcribe/
  gawr-gura-quest-for-bread/pixvael），切换渲染正常
### 22. 点击直达 + 高度自愈（2026-09-17）

- 痛点：macOS 默认"首击只激活窗口"，胶囊点第一下没反应（用户抱怨"点了效果不太好"的根源）
- 修复：objc2 给窗口加 NSNonactivatingPanelMask（non-activating panel），点击直达 webview
  不抢 ZCode 焦点；实测第 1 击即展开、第 2 击收起
- 高度卡死修复：面板展开时 hide→show 会恢复旧 frame 绕过高度缓存，syncHeight 改为
  对比真实 innerHeight 不符即纠正（3s 轮询自愈），实测 172→24 自愈正常
### 23. 显示完整性：响应式主行 + 面板实测高度（2026-09-17）

- 用户反馈：主行右侧文字截断；面板里"输出速度/输出"被遮挡
- 主行：宽度响应式隐藏低优先级段（<392 隐 sparkline+CTX，<352 再隐 Σ），保 ⌄/速率/项目/⚡ 完整
- 面板：废弃固定预算 POP_HEIGHT，renderPop 后按 getBoundingClientRect 实测高度更新窗口；
  窄窗(<392)面板自动切单列；实测 8 项指标+会话芯片全可见
### 24. 显示完整性终版（2026-09-17）

- 主行：padding-right 12 + chevron margin，⌄ 不贴右缘；宽度响应式隐藏低优先级段
- 面板高度：确定性固定值（双列 205 / 窄窗单列 260）+ 实测下限 + overflow 滚动兜底
- 数值防裁：kv 标签 flex 收缩省略号、数值 shrink:0（列 187px vs 最长值 ~90px，几何上无裁切空间）
- 注：截图+AI 判读验证在该环境噪音大（屏幕睡眠/窗口自动收起导致错拍），终版以几何保证+用户目验为准
### 25. 对齐/默认会话修正（2026-09-17，用户截图反馈）

- "太靠右"：窄窗下胶囊与两侧芯片仅 8px 边距显得贴右；改为两侧各留 ~20px（target_w=gap_w−40），
  宽窗时胶囊 410 保持、在空隙内真实居中（实测左右各 147px）
- 默认会话错选：默认"最近活跃"会被后台跑着的其他对话抢占；新增 live_session 偏好持久化——
  用户点选会话后记住（app_prefs），下次打开沿用手动选择；空=自动跟随最近活跃
### 26. 位置锚定左芯片（2026-09-17，红框意图收敛）

- 用户两次反馈"太靠右"；Gemini 测量证实胶囊在空隙内几何居中，但视觉参照是整条
  工具栏——左侧空白大、右贴 GLM 芯片，等分居中必然显得偏右
- 改锚定式布局：胶囊左缘 = 左芯片右缘 + 46px（用户红框标定值），窗口缩放时相对
  左芯片恒定；窄窗自动左移防与右芯片重叠（保 20px）
- 实测：x = 左芯片右缘 + 46 精确命中
### 27. 模型名动态宽度（2026-09-17，长名贴右芯片修复）

- 用户选 ds4.1 等长名模型后胶囊贴住模型选择器：写死的右侧簇宽 532（GLM-5.3-flash 标定）
  在长名时失效（芯片左缘左移）
- 修复：从 ZCode DB 最近的 model_id 取当前模型名长度，动态估算右侧簇宽
  right_zone = clamp(470 + 6.5×字符数, 500, 780)，替代常数 532
- 验证：19字符模型下胶囊右缘距芯片 ~186px；30字符时防重叠夹取仍有余量
### 28. 速率实时化（2026-09-18，"今天不显示速率"修复）

- 根因：速率靠 CostDog 30s 扫描节拍做差分——扫描间隙 tokens 不变→EMA 衰减为 0（idle），
  扫描落地时又虚高跳动；高频对话时被掩盖，低频使用即暴露
- 修复：zcode 源速率直读 ZCode DB 最近 60s 的 output_tokens 总和（tokens/min），
  与 CTX 同一条实时链路，绕开扫描；60s 窗口天然平滑，弃用 EMA
- 实测：部署后立即显示 ▲20/s（当时对话实时流量）
### 29. 速率改为指数衰减窗口（2026-09-18，"一直 30/s"反馈）

- 60s 平均在你"每条~500tok/间隔~15s"的模式下恒为 ~30/s，数字不动被疑为卡死（实为真实平均）
- 改为指数衰减窗口（τ=20s）：rate=Σ(output/τ)·e^(−Δt/τ)，生成完成冲高、停顿自然回落，
  <0.05 tok/s 归零（idle）——有"正在生成"的心跳感且不闪烁
### 30. 下边框被"盖"修复（2026-09-18）

- 用户报胶囊最下面边框被 ZCode 盖住；z 序实测正常（CostDog rank20 在 ZCode rank21 之上）
- 真因：v4 给胶囊加 1px 上下边框（总高 26），syncHeight 仍按 24px 内容计——下边框溢出
  窗口被裁，视觉似"被盖"
- 修复：syncHeight 总高 +2px 边框余量；实测窗口 26px、下边框完整
### 31. 窗口高度全实测（2026-09-18，"还是不完整"根治）

- 用户仍报不完整；部署实测驱动后发现：主行真实渲染高 32px，代码假设 24px——
  窗口恒裁掉底部 8px（下边框+部分内容），此前 +2px 修复不够
- 根治：syncHeight 全实测——主条/弹层高度均按 getBoundingClientRect 真实渲染计算，
  不再假设任何行高/边框占位；窗口永远精确包住内容
- 实测：窗口自动 34px（32 内容+边框），底部完整
### 32. Codex（ChatGPT 桌面端）适配 v1（2026-09-19）

- 宿主确认：Codex Desktop 实为 ChatGPT.app，窗口 owner='ChatGPT'——KNOWN_CLIENTS 加 ChatGPT
- 嵌入几何按宿主分支：ChatGPT 无 ZCode 芯片行，v1 用窗口内右下角 16px；窗口 <450 宽隐藏
- codex 会话指标：解析最近活跃 rollout JSONL（8MB 毫秒级+mtime 缓存）——
  role=user message 为 turn 边界，TTFT=user→首 token_count，模型时长=首 token→turn 末事件，
  工具用时=墙钟−模型，token 分组取 last/total_token_usage，tool_calls 计数 function/custom_call
- 实测（单脚本防焦点抢占）：ChatGPT 前台 → 胶囊精确到达右下角 (1492,1154,410×34)；
  切回 ZCode 自动恢复芯片行锚定位
- 已知项：ChatGPT 前台时点击胶囊展开面板未生效（non-activating panel 与 wry 点击路由待查，
  非 blocking——ZCode 主场景正常）；list_recent_sessions 暂只支持 zcode（codex 芯片行空）
### 33. ChatGPT 宿主配色适配（2026-09-19）

- 本地像素采样（单脚本保前台 + screencapture + NSBitmapImageRep）：ChatGPT 窗口底部
  有白色区域（#FFF/#EDEDED），胶囊落在深色输入框上方——统一深灰胶囊不协调
- 新增 body[data-host="chatgpt"] profile：浅色玻璃 rgba(255,255,255,.85)+blur8、
  白底黑字体系（文字 rgba(0,0,0,.78)、强调 #1a1a1a、命中绿 #15803d、CTX 紫 #7c3aed）、
  面板/会话芯片全套浅色
- 验证：胶囊内采样 rgb(221,221,221) ≈ 浅玻璃(0.85×255)叠深输入框(47×0.15)=224 ✓ 生效
- 注意：Gemini 服务端本轮报位置限制不可用，采样全部本地化（swift+NSBitmapImageRep）
### 34. ChatGPT 皮肤 v3：克隆输入框设计语言（2026-09-19）

- 用户要求"完美融入 codex 皮肤"；v2 半透明白玻璃（采样 221 混合漂移）不够原生
- v3：不透明 #f4f4f4 底（=ChatGPT 输入框底色）、rgba(0,0,0,.06) 微边框、12px pill 圆角、
  微阴影、**纯黑白灰配色**（ChatGPT UI 零彩色：rate 黑粗体、标签 #8f8f8f、无语义彩）
- 面板白底 #fff 同语言
- 采样验证：胶囊 rgb(242,242,242) 精确 = 设计值（不透明无混合漂移），与宿主 #FFF
  的灰阶关系 = 输入框对聊天区的关系
### 35. codex 命中率 4752% 修复（2026-09-19）

- 根因：codex parser 刻意存"未缓存输入"（input−cached，为计价正确），而
  get_live_stats 命中率公式分母当作总输入 → cache_read/input 爆表
- 修复：命中率分母按源分支——codex = cache_read/(input+cache_read)；zcode/claude
  的 input 含缓存，公式不变
- 验证：最近 codex 会话真值 42,112,768/(877,015+42,112,768)=97.96% ✓
### 36. 速率单位 60 倍 bug（2026-09-20，"跑着不显示速率"根治）

- TPS-hold/衰减算法返回 tok/s，直接赋给 tokens/min 字段且前端再 ÷60 → 显示恒 0~1
- 修复：覆盖时 ×60 换算；另确认 sessions 表 UTC 存储+扫描推进正常（伪异常排除）
- 现语义：最近一条真实生成（≥50tok，过滤标题类小请求）的 TPS，生成中实时、
  完成后保持 45s，之后 idle
### 37. cap 胶囊脱管修复 + host→source 映射（2026-09-23）

- 用户报：速率老不显示 + 最小化后胶囊残留桌面
- 根因①：manage_chatgpt_capsule 原在循环尾部，多个 continue 路径（scanned None/
  owner!=ZCode）先于它执行 → ChatGPT 胶囊无人隐藏而残留；其数据 fallback 错源
  （host=chatgpt 找不到同名 source → 取 rows[0] 可能非 codex）→ 速率长期 idle
- 修复：manage 提到循环最前每 tick 无条件执行（内部自查前台）；前端 host→source
  映射 chatgpt→codex（主行与 focus 两处）
- 验证：ZCode 前台 ▲19/s+CTX525k；切 Chrome 0 窗；回 ZCode 1 窗重现
- 教训：app 冷启动需 ~8s 初始化后才可读窗口（4s 时序曾误判回归）
### 38. 方案 A：菜单栏公民形态（2026-09-23，产品方向重构）

- 用户判断：速率栏鸡肋（决策价值低而 bug 最多）+ 嵌入伪装路线维护成本高 → 采纳
- 菜单栏 popover：常驻隐藏 panel 窗口（#panel hash 面板模式），托盘左键切换，
  右上角对齐图标下缘+6px，失焦自动收起；面板=今日花费大字 + 各客户端卡片
  （项目/Σ/CTX/命中）+ 当前会话 8 项指标
- 主胶囊砍速率：▲tok/s → $今日花费（30s 粒度），sparkline 移除；CTX/命中保留
- 托盘左键不再 toggle 主条（由 popover 接管）；dock 停靠逻辑冻结不再迭代标定
- 验证：panel ready 日志 ✓、主胶囊 $19.52+CTX+命中 ✓；popover 点击需用户手验
  （系统状态区图标密集，自动化点击点不中）
### 39. 修正：常驻胶囊为第一公民（2026-09-23）

- 用户纠偏："实时看"=常驻瞥视零操作；popover 主动查询只能作详情入口
- 主行恢复全指标常驻：🐕 | $今日(22.47) | ▲速率(26/s, TPS-hold 稳定版) | 走势图 | 项目 | ⚡90% | CTX 541.9k
- 认知修正：速率此前"鸡肋"的根源是显示不稳定而非无价值——它是唯一真正实时
  跳动的指标（花费分钟级/CTX 会话级），算法稳定后即核心价值
- popover/菜单栏保留为详情入口；停靠胶囊继续常驻
### 40. 洞察面板 v2（2026-09-23，路线 2：惊叹点建在数据上）

- 用户定向：不推宿主团队（"有需求人家自己会做"），火力放数据洞察
- 面板重组：今日花费大字+环比、月预算进度条、自动洞察卡（最多3条：贵项目/模型
  单位成本对比/缓存命中率异常/日环比）、项目 Top3 与模型对比条形、7 天花费
  趋势 sparkline、活跃客户端卡片、当前会话 8 项
- 数据全部复用 get_analytics(today/week)/budget/live_stats/session_metrics，零新增后端
- JS 语法 node --check 通过；渲染需用户点托盘手验（自动化点不中状态区图标）
### 41. 洞察规则 #1：会话烧钱异常（2026-09-23，用户选择 1）

- get_session_cost_alert：最近活跃会话累计 cost vs 近 30 天其他会话平均
- 触发：≥$1 且 ≥3×平均（avg<$0.10 样本不足不触发）
- 前端作为最高优先级洞察卡置顶（💸 图标）
- 真值验证：当前会话 $14.83 vs 平均 $4.78 = 3.1×，恰好触发——面板上即显示
### 42. 屏幕顶边常驻条（2026-09-23，Agent HUD 形态借鉴 v1）

- 动机：用户要"实时看速率"，但胶囊绑死宿主前台（切走即隐藏）——挂机监控场景永远缺席
- 形态（借 Agent HUD）：系统领地常驻——主屏顶边居中 560×28 深色半透明条（blur10），
  always-on-top + set_ignore_cursor_events 鼠标穿透（点击落到下层应用），与前台/最小化无关
- 内容：今日花费(橙) │ ▲速率(活跃源 tok/s，绿=live) │ 各客户端 logo 点(亮=运行中)
  数据=现成 live_stats(2s 轮询)+analytics；#topbar hash 面板模式
- 实测：窗口上屏(680,30,560×28)、材质 rgb(20,20,23) 符合设计、topbar ready 日志 ✓
- 待 v2：配额燃速+耗尽倒计时（BurnRate 语义）、非刘海屏 logo 队列、点击展开详情
### 43. 宠物形态（2026-09-23，最终常驻形态）

- 用户定调：嵌入胶囊停用；常驻形态改为"宠物"——回到 CostDog 灵魂（它本来就是只狗）
- 实现：topbar 窗口参数改 120×120 圆形悬浮球（#pet hash），可拖拽/可点击（交互型，
  不穿透），贴屏幕顶部居中
- 状态=行为：🐕 跑动动画(绿辉光,有实时流量) ⇄ 😴 呼吸睡觉(空闲)；球上徽章=今日
  花费 $；球下 HUD=▲速率 + CTX；2s 轮询
- 嵌入胶囊（ZCode 芯片行/ChatGPT 右下角）停用，dock.rs 保留待复活
- 实测：AX 确认 🐕/$24.23/CTX 537.8k 渲染；窗口 900,30 120×120
### 44. 宠物球黑方块修复（2026-09-23）

- 用户只见黑方块：代码创建的宠物窗口缺 .transparent(true)（主窗口的透明在
  tauri.conf.json 里，builder 创建的要显式设置）；且 macOS 透明窗口需
  macOSPrivateApi: true（conf）+ Cargo tauri features 加 macos-private-api
- 修复后采样：四角 rgb(41,106,159)=真实桌面背景（此前全黑 28,28,28）
- 退役主胶囊窗口（conf 静态创建）setup 中 window.hide()——屏幕只剩宠物球
### 45. 宠物球不可见根治（2026-09-23）

- 透明修复后仍不可见：#pet hash 拼进 Tauri App URL 会破坏页面加载（webview 空白全透明）
- 二分定位法：body 首行注入早期 label 标记脚本 + 红色调试背景——红色出现证明
  加载/检测/透明全通，问题确系 hash URL；改纯 index.html + label 检测（WIN_LABEL==='topbar'）
- 终验：🐕 + $28.49 徽章 + CTX 549.8k 渲染，四角透出真实背景
- 教训：Tauri WebviewUrl::App 不要拼 hash；窗口模式判定用 metadata.currentWindow.label
### 46. 宠物球根治：隔离 pet.html（2026-09-23）

- 根因实锤：主 index.html 的巨型脚本/CSS 在透明窗内不绘制（body 底色能画、
  子元素图层全丢——WKWebView 合成异常）；隔离的 pet.html 同窗同配置完美绘制
- 最终方案：宠物窗口加载独立 embedded/pet.html（自包含：orb 样式+数据轮询脚本，
  $今日徽章/▲速率/CTX/🐕跑😴睡/绿光环 live 态/data-tauri-drag-region 拖拽）
- 教训（重要）：透明悬浮窗一律用独立小 HTML，勿与主应用页共用——主脚本对透明
  合成有干扰且难排查；诊断靠 Read 工具直接看截图（像素采样/AX 都会误导）
### 47. 像素小狗 + 拖拽（2026-09-25）

- Codex CLI 生图不可用（用量上限至 9/28）→ 手工 SVG 像素小狗先行
- pet.html v2：SVG 像素狗（跑态两帧腿部交替+bob 起伏 / 睡态趴卧闭眼），
  状态由实时速率驱动（>0.05 tok/s 跑，否则睡+Zzz 浮动）；拖拽经
  data-tauri-drag-region（#pet/svg/hud 多处）
- 实拍验证：小狗+徽章 $25.70+▲40/s+CTX 535.7k 渲染正常
- 用户指示：下一步用浏览器访问 ChatGPT 生图做真实宠物模型
### 48. ChatGPT 生图宠物皮肤上线（2026-09-25，用户指定流程全通）

- 链路全通：huashu CDP 接管用户 Chrome（隔离 profile ~/.costdog/chrome-profile，
  Chrome 136+ 默认目录禁调试端口→必须独立 user-data-dir）→ 驱动已登录 ChatGPT
  生图（1536×1024）→ canvas+dataURL 触发 Chrome 下载到 ~/Downloads → 接入
- 资产：embedded/sprite.png（原图）、pet-run.png(766×543)/pet-sleep.png(701×380)
  （PIL 白底转透明+包围盒裁剪）
- pet.html v3：img 精灵替换 SVG，跑态 bob 动画、睡态+Zzz 浮动，徽章/HUD 不变
- 实拍：ChatGPT 生成柴犬奔跑帧悬浮 Chrome 之上，▲18/s+CTX 556.9k 实时
- 通道注意：9333 端口被 ego lite 占用（huashu 会误连）；CDP 用 9222+独立 user-data-dir
### 49. 假3D 舞台 + 三宠物形态（2026-09-26）

- 用户要求：假3D 形态 + 多动物宠物；生图通道（CDP 隔离 profile→ChatGPT）已自动化，
  新增橘猫、灰白兔两张 2.5D 精灵图（PIL 白转透明切帧）
- pet.html v4 假3D 舞台：地面椭圆影子与跳跃同步缩放（跳起影子缩小变淡）、
  每 4.2s 左右游走 ±24px 并按方向翻转朝向、跑态 bob/睡态趴卧
- 宠物切换：⇄ 芯片（球左上，无拖拽属性）点击循环 狗→猫→兔；双击被拖拽区吞掉不可用
- 实拍：狗/猫/兔三张奔跑帧渲染确认；影子+游走生效；⇄ 芯片坐标自动点击未验证
  （目标 20×16 太小），留用户真实鼠标验证
### 50. 纯抠图形态 + 头顶数据铭牌（2026-09-26）

- 用户定调：这一层只要动物抠图，数据显示跟在宠物身上
- 移除：地面影子层、独立 HUD 行、原徽章
- 新增：头顶铭牌 .plate（▲速率│$今日│CTX 三合一，深色圆角小牌悬浮宠物头顶，
  随游走/翻转/跳跃一起动）；保留 ⇄ 切换芯片、Zzz（睡态改由铭牌显示 zzz+牌下浮字）
- 窗口仍 120×120 透明；铭牌溢出窗口顶部（top:-16）——透明窗允许内容出界吗？
  实测渲染可见（铭牌在窗口上缘外仍显示）
### 51. 全屏漫游小狗（2026-09-26，"真的小狗在桌面上"）

- 宠物窗口改为全屏透明（铺满主屏）+ 默认整窗穿透；光标守卫线程（80ms 轮询
  NSEvent.mouseLocation，无需权限）光标进小狗范围 ±12px 才解除穿透可交互
- 行为状态机（pet.html v6）：沿屏幕底边散步（随机目标/停顿/张望/转向）、
  agent 有流量时兴奋加速、静默 60s 原地趴睡+Zzz；铭牌（速率/花费/CTX）跟随小狗
- 前端每 tick 上报小狗矩形（set_pet_rect 命令）；objc2 NSEvent.mouseLocation
  读取光标（左下原点→转左上），NSPoint 需手写 Encode/RefEncode（ENCODING_REF）
- 实测：小狗在屏幕右缘漫游、铭牌跟随（▲17/s·$26.06·CTX 692k）、位置随时间变化
### 52. 桌面漫游小狗·最终形态（2026-09-27，回归已验证组件）

- 全屏透明窗方案反复"AX 有 DOM 无像素"（set_size 放大/collection behavior 均
  排除后依旧，间歇性 wry 合成问题）→ 果断回归**已验证渲染**的 150×150 小窗架构
- 漫游改由 Rust 驱动：spawn_roamer 线程沿主屏底边移动窗口本身（狗=窗口），
  walk/pause 状态机+随机掉头+xorshift 随机；小窗天然不挡桌面，穿透问题消失
- pet.html 保留精灵图+铭牌+⇄ 切换；sleeping 作用域 bug 修复
- 实测：窗口 3 秒移动 78px，截屏确认小狗+铭牌在底边散步 ✓
- 教训：透明全屏窗在 wry 上不可靠；"移动小窗"是稳定等效方案
### 53. 渲染间歇 bug 根治（2026-09-27，二分定位）

- 症状：宠物窗 AX 有 DOM 无像素（红底都不画）；全屏/小窗、移动/静止均间歇发作
- 二分定位：pet.html 回滚到 9f2cc99（纯抠图+铭牌版）→ 判空=否渲染恢复！
  凶手=v6 页面重写（.stage absolute+inset:0 / .critter absolute+bottom 布局进入
  透明窗合成死区；漫游 Rust set_position 无辜）
- 最终形态：9f2cc99 页面（居中 flex）+ Rust 漫游（walk/pause/掉头）+ 150×150 小窗
- 实测：漫游中（217,924 → 移动确认）判空=否，狗+铭牌渲染 ✓
- 铁律：透明窗页面禁用 absolute+inset 全铺布局；改布局先小步验证渲染
### 54. 多会话语义：铭牌显示会话名 + 点击切换（2026-09-27）

- 用户问题：多会话/多窗口时宠物显示哪个对话？
- 语义定版：铭牌最前加会话名（蓝色=自动跟随最活跃，橙色=已锁定）；
  点击铭牌循环切换最近 5 个会话，切到最后再点=解锁回自动跟随
- 后端：list_recent_sessions 泛化 codex（rollout 文件名取 id+首行 session_meta 取 cwd
  basename，mtime top5）；CTX 按选中会话取（get_session_metrics sessionId）
- 实拍：铭牌 ZCodeProject·▲/s·$·CTX 渲染确认
### 55. 手动挪动优先于漫游（2026-09-27）

- 用户要求支持手动挪动；拖拽区已有但漫游线程 90ms 会抢回坐标
- 拖动优先：窗口 Moved 事件里对比实际位置与漫游目标（>0.8px 偏差=用户拖）
  → note_user_drag；漫游线程每 tick 检查（3 秒内拖过→以新位置为起点暂停 3s 再继续）
- 实测：拖到 x=1500 → 2.5s 停在 1500（让位）→ 6.5s 走到 1550（新起点续走）✓
### 56. 散步路线抬高到 Dock 之上（2026-09-27）

- 用户：小狗藏在底部任务栏（Dock）后面偷着跑——原路线距屏底 6px，正好钻进 Dock
- Dock 实测 52px 高；路线改为 screenH-150-100，狗全身+铭牌在 Dock 上方行走
- 窗口级截图验证：铭牌+▲/s 渲染正常，位置 y=830
### 57. 可见性根治：白描边+慢步（2026-09-27）

- 用户："刚能看见，跑一跑又不见了"——轨迹正常（窗口级截图渲染恒正常）
- 真凶：棕褐狗走进深色网页背景视觉融合 + 步速快（~160px/s）难以追踪
- 修复：sprite 三层 filter（白色贴边描边+白晕+深投影，任何背景可见）；铭牌白描边；
  散步放慢（步频 90→160ms、步幅减半、停顿 3-7s）
- 实拍：白描边小狗+铭牌，深色背景下轮廓清晰
### 58. 8 帧走路循环——消除"相框感"（2026-09-28）

- 调研 Shimeji 系桌宠："真实感"核心=多帧连续动画（4-8 帧 walk cycle），
  单帧平移永远是"一张图在动"
- ChatGPT 生成 8 帧柴犬侧面步行周期精灵图（2137×218 均分 8 帧，四腿交替清晰）
- pet.html 帧引擎：走=130ms/帧循环 walk-0~7；停=定格第 0 帧；睡=sleep 图；
  猫/兔待补多帧（暂单帧跑图）
- Rust 步频 160ms 与帧率协调（约 2 帧/步）
- 实拍：走路中帧（walk-N 渲染中）确认
