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
