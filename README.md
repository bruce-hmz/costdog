# 🐕 CostDog

**Claude Code & Codex CLI 的成本与资源监控工具**

本地运行，自动解析日志文件，实时统计 token 用量、费用和磁盘写入量。

## 功能

- 📊 **Token 用量追踪** — 按日期/模型/项目汇总 input/output/cache tokens
- 💰 **成本计算** — 基于 OpenRouter 实时价格，精确到每个 session
- 💾 **磁盘写入监控** — 追踪 Write/Edit 工具的磁盘写入量
- ⚠️ **智能预警** — 日费用超标、异常磁盘写入自动告警
- 🖥️ **桌面小窗口** — Tauri 跨平台桌面组件（Mac + Win）
- 🌐 **Web 面板** — 浏览器仪表盘，自动刷新

## 安装

```bash
git clone https://github.com/yourname/costdog.git
cd costdog
npm install
npm run build
npm link  # 全局可用 costdog 命令
```

## 使用

```bash
# 默认显示仪表盘
costdog

# 扫描日志
costdog scan

# 查看模型价格
costdog pricing

# 启动 Web 面板
costdog web
# 浏览器打开 http://localhost:3456
```

## 支持的数据源

### Claude Code
- `~/.claude/projects/*/*.jsonl` — Session 日志（主要数据源）
- 自动解析 token usage、tool calls、磁盘写入

### Codex CLI
- `~/.codex/sessions/*/rollout-*.jsonl` — Session 回放
- `token_count` 事件中的累计用量
- rate_limits 数据

## 环境变量

| 变量 | 说明 |
|---|---|
| `CODEX_HOME` | 覆盖 Codex 配置目录 |
| `COSTDOG_DATA_DIR` | 覆盖 CostDog 数据目录 |
| `COSTDOG_PORT` | Web 面板端口（默认 3456） |

## 价格数据

从 [OpenRouter](https://openrouter.ai) API 实时获取，缓存 24 小时。
支持 339+ 模型，包括 Claude、GPT、Gemini、DeepSeek、GLM 等。

## 技术栈

- TypeScript + Node.js
- better-sqlite3（本地数据库）
- Express（Web 服务器）
- Chalk（CLI 终端颜色）
- Tauri（桌面小窗口，开发中）

## 项目结构

```
src/
├── parsers/
│   ├── claude-code.ts    # Claude Code 日志解析器
│   └── codex.ts          # Codex 日志解析器
├── utils/
│   ├── paths.ts          # 跨平台路径检测
│   └── pricing.ts        # OpenRouter 价格加载 + 成本计算
├── db/
│   └── schema.ts         # SQLite 数据库操作
├── cli/
│   └── index.ts          # CLI 入口
├── web/
│   ├── server.ts         # Express API 服务器
│   └── public/
│       └── index.html    # Web 仪表盘
├── aggregator.ts         # 数据聚合 + 告警
└── types.ts              # TypeScript 类型定义
```

## License

MIT
