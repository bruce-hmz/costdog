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
