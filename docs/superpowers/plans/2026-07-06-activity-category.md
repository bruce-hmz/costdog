# Activity Category (消费类型分类) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 CostDog 桌面 bar 的 detail 面板加一个记账本式的「消费类型」圆环 —— 按 session 把花费归入 9 类(写代码/修Bug/重构/文档/调研/调试/编排/阅读/其他),纯本地规则判定,只改 Rust 侧。

**Architecture:** Rust `scan_claude_sessions` / `parse_codex_rollout` 在既有解析循环里补采 `tool_use` + `usage.server_tool_use` + `gitBranch` → 纯函数 `classify(tool_calls, git_branch, source)` 用规则链判 9 类 → `upsert_session` 写入 `sessions.activity_category` → 新增 `get_cost_by_category` 查询 → `DashboardData.today/week/month/allTime` 各带一份 `byCategory` → bar 前端 SVG donut 渲染。

**Tech Stack:** Rust(Tauri 2, rusqlite 0.31 bundled, serde/serde_json, chrono), SQLite, 纯 SVG + JS(无图表库)。

**Spec:** `docs/superpowers/specs/2026-07-06-activity-category-design.md`(v2.1,已过 Codex 两轮审核)

## Global Constraints

- **本地优先**:零云调用、零 API Key、不上传任何数据。
- **本轮只在 Rust 侧实现**:bar 链路(`src-tauri/src/lib.rs` + `src-tauri/embedded/index.html`)。TS 侧 `src/db/schema.ts` 仅同步 schema 列(不填值、不清值)。
- **类别表 + 阈值 v1 硬编码**(常量数组),不引入用户自定义、不引入跨语言共享 json(YAGNI)。
- **commit 用 conventional commits**:`feat:`/`test:`/`refactor:`/`chore:`。
- **Rust 工具链命令一律在 `src-tauri/` 下执行**(crate 根),TS 命令在仓库根。
- **DB 路径**:`~/.costdog/costdog.sqlite`(macOS:`/Users/<user>/.costdog/costdog.sqlite`)。

---

## File Structure

| 文件 | 责任 | 本轮改动 |
|---|---|---|
| `src-tauri/src/lib.rs` | Rust 全部逻辑(scan/parse/classify/upsert/query/get_data) | 大改 |
| `src-tauri/embedded/index.html` | bar 前端(横条 + detail 面板) | 中改(加圆环) |
| `src/db/schema.ts` | TS 侧 schema 迁移 | 小改(同步 3 列) |
| `docs/superpowers/specs/2026-07-06-activity-category-design.md` | 设计 spec | 不改(已完成) |

`lib.rs` 已 1561 行、承担多个职责(scan + price + alert + query + Tauri commands),本轮**不拆分**(surgical changes),只在其中加 `classify` + 类别常量 + 补采集 + 查询。`classify` 与类别常量作为独立逻辑段集中放置,方便日后随架构统一(§12)整体迁出。

---

## Task 1: `classify` 纯函数 + 类别表 + 单测(TDD 核心)

**Files:**
- Modify: `src-tauri/src/lib.rs`(在 `mod tests` 上方加 `classify` + 常量;在 `mod tests` 内加测试)
- Test: `src-tauri/src/lib.rs` 内 `#[cfg(test)] mod tests`(已存在,L1515)

**Interfaces:**
- Consumes: 无(纯函数)
- Produces:
  - `fn normalize_branch(raw: &str) -> String`
  - `fn classify(tool_calls: &HashMap<String,u64>, git_branch: Option<&str>, source: &str) -> &'static str`(返回 9 类 key 之一;emoji/颜色/中文名是展示映射,放前端 JS —— Task 6 的 `CATS`)

- [ ] **Step 1: 写失败测试 —— 在 `mod tests` 内追加(spec §10 用例表)**

在 `src-tauri/src/lib.rs` 的 `#[cfg(test)] mod tests { ... }` 内追加(保留现有测试):

```rust
    fn tc(pairs: &[(&str, u64)]) -> HashMap<String, u64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn classify_branch_signals() {
        // gitBranch 首段精确匹配,优先于工具占比
        assert_eq!(classify(&tc(&[("Write", 10)]), Some("origin/Fix/login"), "claude-code"), "bugfix");
        assert_eq!(classify(&tc(&[("Write", 10)]), Some("refs/heads/docs/readme"), "claude-code"), "docs");
        assert_eq!(classify(&tc(&[("Edit", 10)]), Some("refactor/api"), "claude-code"), "refactor");
        assert_eq!(classify(&tc(&[("Bash", 10)]), Some("feature/donut"), "claude-code"), "feature");
        // 无斜杠 / 不在集合 → 不命中分支规则
        assert_eq!(classify(&tc(&[("Bash", 6)]), Some("feat-category"), "claude-code"), "debug"); // 落到工具占比
        assert_eq!(classify(&tc(&[("Write", 10)]), Some("main"), "claude-code"), "feature");
    }

    #[test]
    fn classify_source_other_for_no_detail() {
        // opencode/zcode 无工具细节 → other(即使 tool_calls 为空也不算 research)
        assert_eq!(classify(&HashMap::new(), None, "opencode"), "other");
        assert_eq!(classify(&HashMap::new(), None, "zcode"), "other");
        // claude/codex 空工具 → research(纯对话)
        assert_eq!(classify(&HashMap::new(), None, "claude-code"), "research");
        assert_eq!(classify(&HashMap::new(), None, "codex"), "research");
    }

    #[test]
    fn classify_tool_ratios() {
        assert_eq!(classify(&tc(&[("Task", 5), ("Read", 5)]), None, "claude-code"), "agent");
        assert_eq!(classify(&tc(&[("WebSearch", 5), ("Read", 5)]), None, "claude-code"), "research");
        assert_eq!(classify(&tc(&[("Bash", 6), ("Read", 4)]), None, "claude-code"), "debug");
        assert_eq!(classify(&tc(&[("Read", 7), ("Grep", 1), ("Write", 1)]), None, "claude-code"), "explore");
        // Edit 提前 + Edit>Write → bugfix(不是 feature)
        assert_eq!(classify(&tc(&[("Edit", 6), ("Write", 4)]), None, "claude-code"), "bugfix");
        assert_eq!(classify(&tc(&[("Write", 4), ("Read", 6)]), None, "claude-code"), "feature");
    }

    #[test]
    fn classify_boundaries_and_fallback() {
        // 严格 >:恰好 0.40/0.50 不命中
        assert_eq!(classify(&tc(&[("Task", 4), ("Read", 6)]), None, "claude-code"), "other"); // 0.40 不 >0.40
        assert_eq!(classify(&tc(&[("Bash", 5), ("Read", 5)]), None, "claude-code"), "other"); // 0.50 不 >0.50
        // explore 要求 写≤0.20;Write 3/10=0.30 >0.20 → 不命中 explore
        assert_eq!(classify(&tc(&[("Read", 7), ("Write", 3)]), None, "claude-code"), "other");
        // 全是未列名工具 → 兜底 other
        assert_eq!(classify(&tc(&[("TodoWrite", 2), ("Skill", 2)]), None, "claude-code"), "other");
    }
```

- [ ] **Step 2: 跑测试,确认失败**

Run: `cd src-tauri && cargo test classify 2>&1 | tail -20`
Expected: 编译失败,`cannot find function classify`(或 `cannot find type` )。

- [ ] **Step 3: 实现 `normalize_branch` + `classify`**

在 `lib.rs` 内、`#[cfg(test)] mod tests` **之前**加入(spec §5,与 spec 完全一致):

```rust
/// 规范化 git 分支名:trim → lowercase → 去 refs/heads/ / origin/ 前缀 → 取首个 "/" 段。
fn normalize_branch(raw: &str) -> String {
    let s = raw.trim().to_lowercase();
    let s = s.strip_prefix("refs/heads/").unwrap_or(&s);
    let s = s.strip_prefix("origin/").unwrap_or(s);
    s.split('/').next().unwrap_or(s).to_string()
}

/// 判定一个 session 的活动类型。规则链首个命中即返回(spec §5)。
fn classify(tool_calls: &HashMap<String, u64>, git_branch: Option<&str>, source: &str) -> &'static str {
    let has_tool_detail = matches!(source, "claude-code" | "codex");

    // ① gitBranch 首段精确匹配(最可信)
    if let Some(raw) = git_branch {
        let head = normalize_branch(raw);
        match head.as_str() {
            "fix" | "bugfix" | "hotfix" | "patch" => return "bugfix",
            "docs"                                 => return "docs",
            "refactor"                             => return "refactor",
            "feat" | "feature"                     => return "feature",
            _ => {}
        }
    }

    let total: u64 = tool_calls.values().sum();
    let ratio = |name: &str| -> f64 {
        if total == 0 { 0.0 } else { *tool_calls.get(name).unwrap_or(&0) as f64 / total as f64 }
    };

    // ② 工具占比主导
    if !has_tool_detail { return "other"; }
    if total == 0 { return "research"; }
    if ratio("Task") + ratio("Agent") > 0.40 { return "agent"; }
    if ratio("WebSearch") + ratio("WebFetch") > 0.40 { return "research"; }
    if ratio("Bash") > 0.50 { return "debug"; }
    let write_edit = ratio("Write") + ratio("Edit");
    if (ratio("Read") + ratio("Grep") + ratio("Glob") > 0.60) && write_edit <= 0.20 { return "explore"; }
    let (we, ed) = (*tool_calls.get("Write").unwrap_or(&0), *tool_calls.get("Edit").unwrap_or(&0));
    if ratio("Edit") > 0.40 && ed > we { return "bugfix"; }
    if ratio("Write") > 0.30 { return "feature"; }

    // ③ 兜底
    "other"
}
```

- [ ] **Step 4: 跑测试,确认全绿**

Run: `cd src-tauri && cargo test classify 2>&1 | tail -20`
Expected: `test result: ok. 9 passed`(4 个 classify_* 测试函数,每个含多个 assert)。

- [ ] **Step 5: 跑全量测试确保没破坏现有**

Run: `cd src-tauri && cargo test 2>&1 | tail -10`
Expected: 全部 PASS(包括原有 L1518 的测试)。

- [ ] **Step 6: Commit**

```bash
cd src-tauri && git add src/lib.rs && git commit -m "feat(ra): activity classify 规则函数 + 9类 + 单测"
```

---

## Task 2: schema 迁移 —— Rust + TS 加 3 列 + busy_timeout

**Files:**
- Modify: `src-tauri/src/lib.rs`(`ensure_db_exists`,L177–290)
- Modify: `src/db/schema.ts`(`getDb`,L11–97)

**Interfaces:**
- Consumes: 无
- Produces: `sessions` 表新增 `activity_category TEXT` / `tool_calls TEXT` / `git_branch TEXT` 三列;两边写连接设 `busy_timeout`

- [ ] **Step 1: Rust —— `ensure_db_exists` 加 busy_timeout + 3 列幂等迁移**

在 `src-tauri/src/lib.rs` 的 `ensure_db_exists` 内,找到现有 `alert_key` 迁移块(L229 附近)的**幂等模式**照搬。在 `date` 列迁移之后(约 L282 之后、`Ok(conn)` 之前)追加:

```rust
    // busy_timeout:Rust 与 TS 并发写同一 DB 时,ALTER 撞 SQLITE_BUSY 时等待重试
    conn.pragma_update(None, "busy_timeout", "5000").ok();

    // Activity category 迁移:3 个新增列,各幂等
    let need = |conn: &rusqlite::Connection, col: &str| -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = ?1",
            rusqlite::params![col],
            |row| row.get::<_, i64>(0),
        ).unwrap_or(0) == 0
    };
    for (col, ddl) in [
        ("activity_category", "ALTER TABLE sessions ADD COLUMN activity_category TEXT"),
        ("tool_calls",        "ALTER TABLE sessions ADD COLUMN tool_calls TEXT"),
        ("git_branch",        "ALTER TABLE sessions ADD COLUMN git_branch TEXT"),
    ] {
        if need(conn, col) {
            conn.execute(ddl, []).map_err(|e| e.to_string())?;
        }
    }
```

> 注:`busy_timeout` pragma 放在函数靠前(建表之前)更稳。若 L177–185 已有其他 pragma(`journal_mode`/`synchronous`),把 `busy_timeout` 紧跟其后即可。

- [ ] **Step 2: Rust 构建 + 启动一次,触发迁移**

Run: `cd src-tauri && cargo build 2>&1 | tail -5`
Expected: 编译通过。

Run(触发 ensure_db_exists):`cd src-tauri && cargo test 2>&1 | tail -5`(或 `npm run tauri:dev` 启动一次再退出)
Expected: PASS。

- [ ] **Step 3: 验证 Rust 写入了 3 列**

Run: `sqlite3 ~/.costdog/costdog.sqlite "PRAGMA table_info(sessions)" | grep -E "activity_category|tool_calls|git_branch"`
Expected: 3 行输出,各 `TEXT` 类型。

- [ ] **Step 4: TS —— `src/db/schema.ts` 同步 3 列 + busy_timeout**

在 `src/db/schema.ts` 的 `getDb()` 内,`_db.pragma('synchronous = NORMAL');` 之后加:

```ts
  _db.pragma('busy_timeout = 5000');
```

并在现有 `date` 列迁移块之后(约 L88 之后、最后的 `CREATE INDEX idx_sessions_date` 之前)加幂等迁移:

```ts
  const needCol = (col: string) =>
    !(_db.prepare('PRAGMA table_info(sessions)').all() as { name: string }[]).some(c => c.name === col);
  for (const ddl of [
    'ALTER TABLE sessions ADD COLUMN activity_category TEXT',
    'ALTER TABLE sessions ADD COLUMN tool_calls TEXT',
    'ALTER TABLE sessions ADD COLUMN git_branch TEXT',
  ]) {
    if (needCol(ddl.split(' ').slice(-2)[0])) {
      _db.exec(ddl);
    }
  }
```

> TS 的 `upsertSession` 本轮**不动**——不写这 3 列,`ON CONFLICT DO UPDATE` 也不设它们(避免清空 Rust 填的值,spec §7)。

- [ ] **Step 5: TS 构建**

Run: `npm run build 2>&1 | tail -5`
Expected: `tsc` 无报错。

- [ ] **Step 6: 验证两边迁移不冲突**

Run: `npm run dev -- scan 2>&1 | tail -3`(触发 TS 扫描,跑 TS 迁移)→ 再 `sqlite3 ~/.costdog/costdog.sqlite "PRAGMA table_info(sessions)" | grep -c activity_category`
Expected: 输出 `1`(列已存在,TS 迁移幂等,不重复、不报错)。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/lib.rs src/db/schema.ts
git commit -m "feat(ra): sessions 表加 activity_category/tool_calls/git_branch + busy_timeout"
```

---

## Task 3: `SessionData` 加字段 + scan 补采集(tool_use / server_tool_use / gitBranch)

**Files:**
- Modify: `src-tauri/src/lib.rs`(`SessionData` L78–91;`scan_claude_sessions` L327–482;`parse_codex_rollout` L487–560;`scan_zcode_sessions` / `scan_opencode_sessions` 构造点)
- Test: `src-tauri/src/lib.rs` 内 `mod tests`(加集成测试)

**Interfaces:**
- Consumes: Task 1(`classify` 不直接用,但 `SessionData.activity_category` 字段为 Task 4 准备)
- Produces: `SessionData` 新增 3 字段;claude/codex 扫描产出填充了 `tool_calls` / `git_branch` 的 `SessionData`

- [ ] **Step 1: `SessionData` 加 3 字段(L78–91)**

把 `struct SessionData { ... }` 改为(在 `cost: f64,` 之后、闭合 `}` 之前加):

```rust
    cost: f64,
    tool_calls: HashMap<String, u64>,
    git_branch: Option<String>,
    activity_category: String,
}
```

> 此时所有构造 `SessionData { ... }` 的地方会编译失败(缺新字段)。Step 3/4/5 逐个补。

- [ ] **Step 2: 写失败集成测试 —— claude 扫描采集 tool_use + server_tool_use**

在 `mod tests` 内加(用 `include_str!` 或临时文件 fixture;此处用 `serde_json` 构造内存 jsonl 写临时文件):

```rust
    #[test]
    fn scan_claude_collects_tools_and_branch() {
        use std::io::Write;
        // 构造 2 行 jsonl:assistant 消息带 tool_use(Write) + server_tool_use(web_search) + gitBranch
        let line1 = serde_json::json!({
            "type":"user","sessionId":"s1","timestamp":"2026-07-06T10:00:00Z",
            "cwd":"/tmp/proj","gitBranch":"feature/x"
        }).to_string();
        let line2 = serde_json::json!({
            "type":"assistant","sessionId":"s1","timestamp":"2026-07-06T10:01:00Z",
            "message":{
                "model":"claude-sonnet-4","usage":{
                    "input_tokens":100,"output_tokens":50,"cache_read_input_tokens":10,
                    "server_tool_use":{"web_search_requests":3}
                },
                "content":[
                    {"type":"tool_use","name":"Write","input":{"content":"x"}},
                    {"type":"tool_use","name":"Read","input":{}}
                ]
            }
        }).to_string();
        let dir = std::env::temp_dir().join("costdog_test_scan");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("session.jsonl");
        let mut fh = std::fs::File::create(&f).unwrap();
        writeln!(fh, "{}", line1).unwrap();
        writeln!(fh, "{}", line2).unwrap();

        // 直接调 parse_jsonl 片段 —— 若 scan_claude_sessions 未暴露单文件入口,
        // 把其内部解析抽成 fn parse_claude_jsonl(path: &Path) -> Vec<SessionData> 后调用。
        let got = parse_claude_jsonl(&f);
        assert_eq!(got.len(), 1);
        let s = &got[0];
        assert_eq!(s.git_branch.as_deref(), Some("feature/x"));
        assert_eq!(*s.tool_calls.get("Write").unwrap_or(&0), 1);
        assert_eq!(*s.tool_calls.get("Read").unwrap_or(&0), 1);
        assert_eq!(*s.tool_calls.get("WebSearch").unwrap_or(&0), 3); // 来自 server_tool_use
    }
```

> 若 `scan_claude_sessions` 当前没有可单测的「解析单文件」函数,Step 3 抽出 `pub fn parse_claude_jsonl(path: &std::path::Path) -> Vec<SessionData>`,`scan_claude_sessions` 内部 walk 时调用它。

- [ ] **Step 3: 抽出 + 改 `scan_claude_sessions`,在 assistant 分支补采集**

把 `scan_claude_sessions` 内「读单个 jsonl」那段(L357–478 的 `if let Ok(file) = fs::File::open(...)` 块)抽成独立函数,并在 `record_type == "assistant"` 分支里(L437–458)**追加** content 遍历 + server_tool_use 采集 + gitBranch 读取:

```rust
fn parse_claude_jsonl(file_path: &std::path::Path) -> Vec<SessionData> {
    // ... 保留原 walk 内部的 (session_id, date) 分桶逻辑 ...
    // 初始化 bucket 时 SessionData 多 3 字段:tool_calls: HashMap::new(), git_branch: None, activity_category: String::new()

    // 在每行解析里:
    // 1) gitBranch(取首个非空)
    if entry.git_branch.is_none() {
        if let Some(b) = data["gitBranch"].as_str() {
            if !b.is_empty() { entry.git_branch = Some(b.to_string()); }
        }
    }
    // 2) assistant 分支:在原有 token 采集之后,追加
    if record_type == "assistant" {
        let usage = &data["message"]["usage"];
        // ... 原有 input/output/cache 采集保留 ...
        // server_tool_use → WebSearch / WebFetch 计数
        let stu = &usage["server_tool_use"];
        if let Some(ws) = stu["web_search_requests"].as_u64() {
            *entry.tool_calls.entry("WebSearch".to_string()).or_insert(0) += ws;
        }
        if let Some(wf) = stu["web_fetch_requests"].as_u64() {
            *entry.tool_calls.entry("WebFetch".to_string()).or_insert(0) += wf;
        }
        // content[].tool_use → 工具计数 + disk_write_bytes(原有逻辑保留)
        if let Some(blocks) = data["message"]["content"].as_array() {
            for b in blocks {
                if b["type"].as_str() == Some("tool_use") {
                    if let Some(name) = b["name"].as_str() {
                        *entry.tool_calls.entry(name.to_string()).or_insert(0) += 1;
                    }
                }
            }
        }
    }
}
```

> 完整实现 = 把原 `scan_claude_sessions` 的桶初始化 + 上述采集合并;`scan_claude_sessions` 改为 walk + 调 `parse_claude_jsonl`。

- [ ] **Step 4: codex 补 tool_call 采集**

`parse_codex_rollout`(L487–560)在 `event_msg` 分支里,除 `token_count` 外,补 `function_call` / `tool_call` 的 name 计数(TS `parsers/codex.ts:99-108` 已有先例):

```rust
        } else if rtype == "event_msg" {
            if payload["type"].as_str() == Some("token_count") {
                // ... 原有 token 采集 ...
            } else if payload["type"].as_str() == Some("function_call")
                   || payload["type"].as_str() == Some("tool_call") {
                if let Some(name) = payload["name"].as_str() {
                    *tool_calls.entry(name.to_string()).or_insert(0) += 1;  // tool_calls 是 fn 内局部 HashMap
                }
            }
        }
```

> codex 无 `gitBranch`、无 `server_tool_use`,这两项对 codex 不采集。codex 的 `SessionData` 构造:`tool_calls`(局部 map)、`git_branch: None`、`activity_category: String::new()`。

- [ ] **Step 5: zcode / opencode 的 `SessionData` 构造点补默认值**

`scan_zcode_sessions` / `scan_opencode_sessions` 里所有 `SessionData { ... }` 构造点,补:
```rust
            tool_calls: HashMap::new(),
            git_branch: None,
            activity_category: String::new(),
```
> opencode/zcode 不采集工具(无细节),`tool_calls` 空 map、`activity_category` 由 Task 4 的 `classify` 在 upsert 前判为 `other`(因 source 不在 claude/codex)。

- [ ] **Step 6: 跑测试 + 构建**

Run: `cd src-tauri && cargo test 2>&1 | tail -10`
Expected: 全 PASS(含新加的 `scan_claude_collects_tools_and_branch`)。

Run: `cd src-tauri && cargo build 2>&1 | tail -5`
Expected: 编译通过(所有 `SessionData { ... }` 构造点都已补字段)。

- [ ] **Step 7: Commit**

```bash
cd src-tauri && git add src/lib.rs && git commit -m "feat(ra): scan 补采 tool_use/server_tool_use/gitBranch + SessionData 扩字段"
```

---

## Task 4: `upsert_session` 按 source 写 + `full_scan` 调 `classify`

**Files:**
- Modify: `src-tauri/src/lib.rs`(`upsert_session` L1049–1081;`full_scan` L1118–1166)

**Interfaces:**
- Consumes: Task 1(`classify`)+ Task 2(列)+ Task 3(`SessionData` 字段)
- Produces: 扫描后 `sessions.activity_category` / `tool_calls` / `git_branch` 有值

- [ ] **Step 1: 改 `upsert_session` —— 加 3 列,`tool_calls` 按 source 写 NULL/JSON**

把 `upsert_session`(L1049)的 SQL 改为:

```rust
fn upsert_session(conn: &rusqlite::Connection, session: &SessionData) -> Result<(), String> {
    // tool_calls 按 source 决定:无细节 → NULL;有细节 → JSON
    let tool_calls_json: Option<String> = match session.source.as_str() {
        "opencode" | "zcode" => None,
        _ => Some(serde_json::to_string(&session.tool_calls).unwrap_or_else(|_| "{}".to_string())),
    };
    conn.execute(
        "INSERT INTO sessions (session_id, source, date, model, project, start_time, end_time,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            reasoning_tokens, disk_write_bytes, cost, activity_category, tool_calls, git_branch)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(session_id, source, date) DO UPDATE SET
            model=excluded.model, end_time=excluded.end_time,
            input_tokens=excluded.input_tokens, output_tokens=excluded.output_tokens,
            cache_read_tokens=excluded.cache_read_tokens,
            cache_creation_tokens=excluded.cache_creation_tokens,
            reasoning_tokens=excluded.reasoning_tokens,
            disk_write_bytes=excluded.disk_write_bytes, cost=excluded.cost,
            activity_category=excluded.activity_category,
            tool_calls=excluded.tool_calls,
            git_branch=excluded.git_branch,
            scanned_at=datetime('now')",
        rusqlite::params![
            session.session_id, session.source, session.date, session.model, session.project,
            session.start_time, session.end_time, session.input_tokens,
            session.output_tokens, session.cache_read_tokens,
            session.cache_creation_tokens, session.reasoning_tokens,
            session.disk_write_bytes, session.cost,
            session.activity_category,
            tool_calls_json,   // Option<String> → NULL when None
            session.git_branch,
        ],
    ).map_err(|e| e.to_string())?;
    Ok(())
}
```

> 关键(spec §7 复审新发现②):`tool_calls_json` 是 `Option<String>`,`rusqlite` 会把 `None` 写成 SQL `NULL`,把 `Some(s)` 写成字符串。opencode/zcode → `NULL`,claude/codex 空 map → `"{}"`。

- [ ] **Step 2: 改 `full_scan` —— upsert 前先 `classify` 填 `activity_category`**

`full_scan`(L1118)里调用 `upsert_session(...)` 之前,把每个 session 算出分类。找到 `for session in &all_sessions { ... upsert_session ... }`(或类似循环),在 upsert 前加:

```rust
    for s in &mut all_sessions {
        s.activity_category = classify(&s.tool_calls, s.git_branch.as_deref(), &s.source).to_string();
    }
    for s in &all_sessions {
        upsert_session(&conn, s)?;
    }
```

> 注意:需要 `&mut all_sessions` 先算分类,再 `&all_sessions` upsert(借用检查)。若 `full_scan` 当前签名不允许 mut,把分类算在构造 `SessionData` 时(scan 内)亦可——但放在 `full_scan` 集中调更清晰,且 `classify` 是纯函数。

- [ ] **Step 3: 构建 + 触发扫描 + 验证写入**

Run: `cd src-tauri && cargo build 2>&1 | tail -5`
Expected: 编译通过。

Run(触发扫描):`cd src-tauri && cargo test 2>&1 | tail -3` 或 `npm run tauri:dev`(启动一次让 `full_scan` 跑)。

Run: `sqlite3 ~/.costdog/costdog.sqlite "SELECT activity_category, COUNT(*) FROM sessions GROUP BY 1 ORDER BY 2 DESC"`
Expected: 至少有 `feature`/`debug`/`research`/`other` 等行 + 计数(取决于你的真实日志)。

Run: `sqlite3 ~/.costdog/costdog.sqlite "SELECT tool_calls, git_branch FROM sessions WHERE source='claude-code' LIMIT 3"`
Expected: `tool_calls` 是 JSON 串(如 `{"Write":3,"Bash":8,"WebSearch":2}`),`git_branch` 有值或 NULL。

- [ ] **Step 4: Commit**

```bash
cd src-tauri && git add src/lib.rs && git commit -m "feat(ra): upsert 按 source 写 tool_calls + full_scan 调 classify 填 activity_category"
```

---

## Task 5: 查询层 —— `get_cost_by_category` + `DailySummary.by_category` + `RecentSession` + `get_data`

**Files:**
- Modify: `src-tauri/src/lib.rs`(`CategoryBreakdown` 新结构;`DailySummary` L31–38;`RecentSession` L40–53;`get_cost_by_category` 新函数;`get_top_models` L1194–1213 统一口径;`get_data` L1239–1329)

**Interfaces:**
- Consumes: Task 2(列)+ Task 4(数据)
- Produces: `get_data` command 返回的 JSON 里,`today/week/month/allTime` 各带 `byCategory: [{key,cost,tokens,sessions}]`;`recentSessions[].activityCategory` 有值

- [ ] **Step 1: 加 `CategoryBreakdown` struct + `DailySummary` 加字段**

在 `DailySummary` 定义(L31)旁加 struct,并给 `DailySummary` 加 `by_category` 字段:

```rust
#[derive(Debug, Serialize, Deserialize)]
struct CategoryBreakdown {
    key: String,
    cost: f64,
    tokens: u64,
    sessions: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct DailySummary {
    date: String,
    sessions: u64,
    #[serde(rename = "tokenUsage")]
    token_usage: TokenUsage,
    cost: f64,
    #[serde(rename = "diskWriteBytes")]
    disk_write_bytes: u64,
    #[serde(rename = "topModels")]
    top_models: Vec<TopModel>,
    #[serde(rename = "byCategory")]
    by_category: Vec<CategoryBreakdown>,
}
```

- [ ] **Step 2: `RecentSession` 加 `activity_category`**

`RecentSession`(L40)加字段:
```rust
    #[serde(rename = "activityCategory")]
    activity_category: Option<String>,
```

- [ ] **Step 3: 加 `get_cost_by_category` + `get_top_models` 统一到 `date`**

在 `get_top_models`(L1194)旁加(spec §8.1-6,用 `COALESCE(NULLIF(...),'other')` 归 other,不过滤):

```rust
fn get_cost_by_category(conn: &rusqlite::Connection, start: &str, end: &str) -> Result<Vec<CategoryBreakdown>, String> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(NULLIF(activity_category,''),'other') AS key,
                COUNT(*) AS sessions,
                COALESCE(SUM(cost), 0) AS cost,
                COALESCE(SUM(input_tokens + output_tokens + cache_read_tokens), 0) AS tokens
         FROM sessions
         WHERE date >= ? AND date <= ?
         GROUP BY COALESCE(NULLIF(activity_category,''),'other')
         ORDER BY cost DESC"
    ).map_err(|e| e.to_string())?;
    let rows = stmt.query_map(rusqlite::params![start, end], |row| {
        Ok(CategoryBreakdown {
            key: row.get::<_, String>(0)?,
            sessions: row.get::<_, u64>(1)?,
            cost: row.get::<_, f64>(2)?,
            tokens: row.get::<_, u64>(3)?,
        })
    }).map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}
```

同时把 `get_top_models`(L1200)的 `WHERE date(start_time) >= ? AND date(start_time) <= ?` 改为 `WHERE date >= ? AND date <= ?`(spec §8.1-6 统一口径)。

- [ ] **Step 4: `get_data` —— `by_category` 填进四个周期 + `RecentSession` SELECT 加列**

`get_data`(L1239)里:
1. 四个周期各调一次 `get_cost_by_category(&conn, &start, &end)?`。
2. `to_daily_summary` 闭包里加 `by_category` 参数/字段。
3. `recent_sessions` 的 SELECT(L1260)加 `activity_category` 列,`query_map` 里映射 `activity_category: row.get::<_, Option<String>>(N)?`。

具体:把 `to_daily_summary` 改为接收 `by_category: Vec<CategoryBreakdown>`,并在四个 `DailySummary { ... }` 构造点填入对应周期的查询结果;`recent_sessions` 的 SQL 改为:

```rust
    let mut stmt = conn.prepare(
        "SELECT session_id, source, model, project, start_time, end_time,
                input_tokens, output_tokens, cache_read_tokens, cost, disk_write_bytes,
                activity_category
        FROM sessions ORDER BY date DESC, start_time DESC LIMIT 20"
    ).map_err(|e| e.to_string())?;
    let recent_sessions: Vec<RecentSession> = stmt.query_map([], |row| {
        Ok(RecentSession {
            session_id: row.get(0)?,
            source: row.get(1)?,
            model: row.get(2)?,
            project: row.get(3)?,
            start_time: row.get(4)?,
            end_time: row.get(5)?,
            input_tokens: row.get(6)?,
            output_tokens: row.get(7)?,
            cache_read_tokens: row.get(8)?,
            cost: row.get(9)?,
            disk_write_bytes: row.get(10)?,
            activity_category: row.get::<_, Option<String>>(11)?,
        })
    }).map_err(|e| e.to_string())?.filter_map(|r| r.ok()).collect();
```

- [ ] **Step 5: 构建 + 跑全量测试**

Run: `cd src-tauri && cargo test 2>&1 | tail -10`
Expected: 全 PASS。

Run: `cd src-tauri && cargo build 2>&1 | tail -5`
Expected: 编译通过。

- [ ] **Step 6: 验证 `get_data` 输出含 `byCategory`**

启动 bar(`npm run tauri:dev`),或在 `get_data` 末尾临时加 `eprintln!("{}", serde_json::to_string_pretty(&data).unwrap());` 后跑一次 `cargo test`,检查 stdout:
Expected: JSON 里 `today`/`week`/`month`/`allTime` 各有 `"byCategory":[...]`,`recentSessions[].activityCategory` 有值。验证后删掉临时 `eprintln!`。

- [ ] **Step 7: Commit**

```bash
cd src-tauri && git add src/lib.rs && git commit -m "feat(ra): get_cost_by_category + byCategory 入 DailySummary + RecentSession 带分类 + top_models 统一 date 口径"
```

---

## Task 6: bar 前端圆环 + session 行 emoji

**Files:**
- Modify: `src-tauri/embedded/index.html`(detail 面板 HTML + CSS + `render()`)

**Interfaces:**
- Consumes: Task 5(`d.byCategory`、`s.activityCategory`)
- Produces: detail 面板显示「消费类型」圆环 + 各类条形;session 列表每行带类别 emoji

- [ ] **Step 1: 加圆环区块的 HTML(在统计卡 `.grid` 之后、`Top Models` 之前)**

在 `embedded/index.html` 找到 `<div id="dm"></div>`(Top Models 容器)所在的那段(L91–92 附近),在它**之前**插入:

```html
  <div class="st">消费类型</div>
  <div class="cat">
    <svg id="catdonut" viewBox="0 0 42 42" width="120" height="120" style="flex-shrink:0">
      <circle cx="21" cy="21" r="15.915" fill="none" stroke="var(--card)" stroke-width="6"></circle>
      <g id="catsegs"></g>
      <text x="21" y="20" text-anchor="middle" font-size="6" fill="var(--dim)" id="catl">TOTAL</text>
      <text x="21" y="28" text-anchor="middle" font-size="7" font-weight="700" fill="var(--text)" id="catv">$0</text>
    </svg>
    <div id="catrows" style="flex:1;min-width:0"></div>
  </div>
```

- [ ] **Step 2: 加 CSS(在 `<style>` 末尾,贴合现有 Catppuccin 变量)**

```css
.cat{display:flex;gap:10px;align-items:center;background:var(--card);border:1px solid var(--border);border-radius:6px;padding:8px;margin-bottom:8px}
.cr{display:flex;align-items:center;gap:5px;font-size:10px;padding:2px 0;white-space:nowrap}
.ce{font-size:11px;width:14px}
.cn{flex:1;color:var(--text);overflow:hidden;text-overflow:ellipsis}
.cv{color:var(--dim);font-size:9px}
.cb{height:4px;border-radius:2px;min-width:2px;margin-left:4px}
```

- [ ] **Step 3: 加类别常量 + 圆环渲染函数(在 `<script>` 内 `render()` 之前)**

```javascript
const CATS={feature:['✨','#89b4fa'],bugfix:['🐛','#f38ba8'],refactor:['♻️','#f9e2af'],docs:['📚','#a6e3a1'],research:['🔍','#cba6f7'],debug:['🧪','#fab387'],agent:['🤖','#94e2d5'],explore:['📖','#74c7ec'],other:['⚪','#6c7086']};
function renderCat(list,totalCost){
  const seg=document.getElementById('catsegs'); const rows=document.getElementById('catrows');
  document.getElementById('catv').textContent=fC(totalCost);
  if(!list||!list.length){seg.innerHTML='';rows.innerHTML='<div style="color:var(--dim);font-size:10px">No data</div>';return;}
  const top=list.slice(0,5);
  const rest=list.slice(5).reduce((a,b)=>a+b.cost,0);
  const merged=rest>0?[...top,{key:'other',cost:rest,sessions:0,tokens:0}]:top;  // 超过5类合并为 other
  // donut arcs
  let off=25,sh=''; const tot=merged.reduce((a,b)=>a+b.cost,0)||1;
  merged.forEach(c=>{const pct=c.cost/tot*100;if(pct<0.5)return;const col=(CATS[c.key]||CATS.other)[1];sh+=`<circle cx="21" cy="21" r="15.915" fill="none" stroke="${col}" stroke-width="6" stroke-dasharray="${pct} ${100-pct}" stroke-dashoffset="${off}"></circle>`;off-=pct;});
  seg.innerHTML=sh;
  // rows
  let rh='';merged.forEach(c=>{const[em,col]=CATS[c.key]||CATS.other;const pct=(c.cost/(tot||1)*100);rh+=`<div class="cr"><span class="ce">${em}</span><span class="cn">${ {feature:'编码',bugfix:'Bug',refactor:'重构',docs:'文档',research:'调研',debug:'调试',agent:'编排',explore:'阅读',other:'其他'}[c.key]}</span><span class="cb" style="background:${col};width:${Math.max(8,pct*0.4)}px"></span><span class="cv">${fC(c.cost)} ${pct.toFixed(0)}%</span></div>`;});
  rows.innerHTML=rh;
}
```

- [ ] **Step 4: 在 `render()` 里调用圆环 + session 行加 emoji**

`render()` 里 `const d=D[M[R]];` 之后,在「Detail panel」段(L128 `document.getElementById('ds')...` 附近)加:

```javascript
  renderCat(d.byCategory||[], d.cost||0);
```

session 列表渲染(L137 附近 `ss.slice(0,15).forEach`)里,在每行 `<td>` 开头加类别 emoji。把原:
```javascript
th+='<tr><td><span class="badge '+bc+'">'+bl+'</span></td>...
```
改为:
```javascript
const catEm=(CATS[s.activityCategory]||CATS.other)[0];
th+='<tr><td><span class="badge '+bc+'">'+bl+'</span></td><td>'+catEm+'</td>...
```
并在表头(L89 附近 `<tr>...<th>Source</th>...`)加一个 `<th></th>` 列对应 emoji。

- [ ] **Step 5: 手动验证 bar 圆环**

Run: `npm run tauri:dev`
Expected: bar 启动 → 展开 detail → 看到「消费类型」圆环(各类彩色弧 + 中心总 cost)+ 右侧各类条形;session 列表每行有类别 emoji。切换 Today/7D/30D/All 圆环随之变。

- [ ] **Step 6: 视觉验收(spec §9 四种情形)**

手动检查:
- **空态**:无分类数据时(新装、无日志)显示 `No data`,不崩。
- **1 类**:只有 other 时圆环单色。
- **9 类**:超过 5 类时只显 Top 5 + 合并的 other,不溢出 410px。
- **长金额**:`$1234.56` 不撑破布局;**低成本** `$0.0001` 正常显示。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/embedded/index.html
git commit -m "feat(ra): bar detail 加消费类型圆环 + session 行类别 emoji"
```

---

## Task 7: 端到端集成验证 + 文档收尾

**Files:**
- 无代码改动;验证 + 文档

- [ ] **Step 1: 全量测试 + 构建**

Run: `cd src-tauri && cargo test 2>&1 | tail -10` → 全 PASS
Run: `cd src-tauri && cargo build --release 2>&1 | tail -5` → 编译通过
Run: `npm run build 2>&1 | tail -5` → TS 无报错

- [ ] **Step 2: 端到端 —— 真实日志扫描 → 圆环显示**

Run: `npm run tauri:dev`
- bar 自动扫描(30s)或点 ⟳ Scan Now
- 展开 detail,确认圆环数据与 `sqlite3 ~/.costdog/costdog.sqlite "SELECT activity_category, SUM(cost) FROM sessions WHERE date>=date('now','localtime') GROUP BY 1"` 结果一致。

- [ ] **Step 3: 抽查几个 session 的分类是否合理(可解释性)**

Run: `sqlite3 ~/.costdog/costdog.sqlite "SELECT activity_category, tool_calls, git_branch, substr(project,1,20) FROM sessions WHERE activity_category!='other' ORDER BY RANDOM() LIMIT 10"`
逐条人工核对:tool_calls 构成 + git_branch 是否能解释该分类(例:`feature` 类应有 Write/feature 分支;`debug` 类应 Bash 主导)。若发现系统性误分,记录到 spec §11 限制表,校准阈值(改 Task 1 的 `classify` + 更新单测)。

- [ ] **Step 4: opencode/zcode 验证归 other**

如果你机器上有 opencode/zcode 日志:
Run: `sqlite3 ~/.costdog/costdog.sqlite "SELECT source, activity_category, COUNT(*) FROM sessions GROUP BY source, activity_category"`
Expected: opencode/zcode 行的 `activity_category` 为 `other` 或空(COALESCE 归 other);`tool_calls` 为 NULL。

- [ ] **Step 5: 更新项目 README / CLAUDE.md 记录新功能**

在 `README.md` 的功能列表加一行「🏷️ **消费类型分类** —— 按 session 自动归类 9 类活动,bar 圆环可视化」。

- [ ] **Step 6: 最终 Commit**

```bash
git add README.md
git commit -m "docs(ra): README 记录消费类型分类功能"
```

---

## 验收标准(Definition of Done)

- [ ] `cargo test` 全绿(含 classify 9 类 + 边界 + scan 采集集成测试)
- [ ] bar detail 面板显示圆环,随 Today/7D/30D/All 切换
- [ ] `sessions.activity_category` 有值;opencode/zcode 归 other
- [ ] TS 侧 schema 同步 3 列,`upsertSession` 不写不清新列
- [ ] 只挂 bar(不跑 CLI)也能看到分类 —— Rust 链路自洽
- [ ] spec §14 的 15 条 Codex 发现全部落实

## 风险与回退

- **阈值不准**:Task 7 Step 3 抽查发现系统性误分 → 调 `classify` 阈值(单测保护),无需动 schema/数据。
- **codex tool_call 字段路径**:Task 3 Step 4 的 `payload["name"]` 若对不上真实 rollout,以真实 JSONL 为准(在 `parse_codex_rollout` 里 `eprintln!` 调试一次)。
- **双迁移冲突**:Task 2 的 busy_timeout + 幂等已覆盖;若仍撞锁,把 ALTER 包进 `unchecked_transaction`。
