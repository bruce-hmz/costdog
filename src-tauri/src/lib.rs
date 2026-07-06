use tauri::{Manager, Emitter};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::fs;
use std::io::{BufRead, BufReader};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
struct TokenUsage {
    #[serde(rename = "inputTokens")]
    input_tokens: u64,
    #[serde(rename = "outputTokens")]
    output_tokens: u64,
    #[serde(rename = "cacheReadTokens")]
    cache_read_tokens: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct TopModel {
    model: String,
    calls: u64,
    cost: f64,
}

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

#[derive(Debug, Serialize, Deserialize)]
struct RecentSession {
    session_id: String,
    source: String,
    model: Option<String>,
    project: Option<String>,
    start_time: Option<String>,
    end_time: Option<String>,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cost: f64,
    disk_write_bytes: u64,
    #[serde(rename = "activityCategory")]
    activity_category: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Alert {
    level: String,
    message: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DashboardData {
    today: DailySummary,
    week: DailySummary,
    month: DailySummary,
    #[serde(rename = "allTime")]
    all_time: DailySummary,
    #[serde(rename = "recentSessions")]
    recent_sessions: Vec<RecentSession>,
    alerts: Vec<Alert>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionData {
    session_id: String,
    source: String,
    date: String,
    model: String,
    project: String,
    start_time: String,
    end_time: String,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    reasoning_tokens: u64,
    disk_write_bytes: u64,
    cost: f64,
    tool_calls: HashMap<String, u64>,
    git_branch: Option<String>,
    activity_category: String,
}

fn get_costdog_db_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let data_dir = std::env::var("COSTDOG_DATA_DIR")
        .unwrap_or_else(|_| home.join(".costdog").to_string_lossy().to_string());
    PathBuf::from(data_dir).join("costdog.sqlite")
}

fn get_claude_sessions_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".claude").join("projects")
}

fn get_codex_sessions_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let codex_home = std::env::var("CODEX_HOME")
        .unwrap_or_else(|_| home.join(".codex").to_string_lossy().to_string());
    PathBuf::from(codex_home).join("sessions")
}

// ZCode CLI stores its SQLite DB at ~/.zcode/cli/db/db.sqlite (ZCODE_HOME overrides ~/.zcode).
fn get_zcode_db_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let zcode_home = std::env::var("ZCODE_HOME")
        .unwrap_or_else(|_| home.join(".zcode").to_string_lossy().to_string());
    PathBuf::from(zcode_home).join("cli").join("db").join("db.sqlite")
}

// OpenCode stores its DB at ~/.local/share/opencode/opencode.db.
// OPENCODE_DB may be an absolute path to the file; XDG_DATA_HOME overrides the base dir.
fn get_opencode_db_path() -> PathBuf {
    if let Ok(v) = std::env::var("OPENCODE_DB") {
        let p = PathBuf::from(&v);
        if p.is_absolute() {
            return p;
        }
    }
    let base = match std::env::var("XDG_DATA_HOME") {
        Ok(xdg) if !xdg.is_empty() => PathBuf::from(xdg),
        _ => {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            home.join(".local").join("share")
        }
    };
    base.join("opencode").join("opencode.db")
}

/// Local YYYY-MM-DD of a millisecond epoch timestamp (so "today" matches the user's clock).
fn local_date_from_ms(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn iso_from_ms(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

/// Open a SQLite DB read-only with a busy_timeout so a running app is never blocked.
/// Returns None if the file is missing, can't be opened, or lacks `table`.
fn open_readonly_db(path: &PathBuf, table: &str) -> Option<rusqlite::Connection> {
    if !path.exists() {
        return None;
    }
    use rusqlite::OpenFlags;
    let conn = rusqlite::Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    conn.busy_timeout(std::time::Duration::from_millis(5000)).ok()?;
    // Confirm the expected table exists — degrade to "no data" on older/other schemas.
    let has_table: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            rusqlite::params![table],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if has_table == 0 {
        return None;
    }
    Some(conn)
}

fn ensure_db_exists() -> Result<rusqlite::Connection, String> {
    let db_path = get_costdog_db_path();
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let conn = rusqlite::Connection::open(&db_path).map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "WAL").map_err(|e| e.to_string())?;
    conn.pragma_update(None, "synchronous", "NORMAL").map_err(|e| e.to_string())?;
    // busy_timeout: Rust 与 TS 并发写同一 DB 时,ALTER 撞 SQLITE_BUSY 时等待重试
    conn.pragma_update(None, "busy_timeout", "5000").ok();

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
            session_id TEXT NOT NULL,
            source TEXT NOT NULL,
            date TEXT NOT NULL DEFAULT '',
            model TEXT,
            project TEXT,
            start_time TEXT,
            end_time TEXT,
            input_tokens INTEGER DEFAULT 0,
            output_tokens INTEGER DEFAULT 0,
            cache_read_tokens INTEGER DEFAULT 0,
            cache_creation_tokens INTEGER DEFAULT 0,
            reasoning_output_tokens INTEGER DEFAULT 0,
            disk_write_bytes INTEGER DEFAULT 0,
            cost REAL DEFAULT 0,
            scanned_at TEXT DEFAULT (datetime('now')),
            PRIMARY KEY (session_id, source, date)
        );

        CREATE INDEX IF NOT EXISTS idx_sessions_start ON sessions(start_time);
        CREATE INDEX IF NOT EXISTS idx_sessions_source ON sessions(source);
        CREATE INDEX IF NOT EXISTS idx_sessions_model ON sessions(model);

        CREATE TABLE IF NOT EXISTS alerts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            level TEXT NOT NULL,
            message TEXT NOT NULL,
            alert_key TEXT,
            timestamp TEXT DEFAULT (datetime('now')),
            dismissed INTEGER DEFAULT 0
        );"
    ).map_err(|e| e.to_string())?;

    // Drop sessions that logged no token usage (opened-but-unused sessions, or models
    // that don't report usage) — they are 0-cost noise. Also enforced at scan time.
    conn.execute(
        "DELETE FROM sessions WHERE input_tokens=0 AND output_tokens=0 AND cache_read_tokens=0 AND cache_creation_tokens=0 AND reasoning_output_tokens=0",
        [],
    )
    .map_err(|e| e.to_string())?;

    // Migration: CREATE TABLE IF NOT EXISTS won't add alert_key to an existing table.
    let has_alert_key: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('alerts') WHERE name = 'alert_key'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if has_alert_key == 0 {
        conn.execute("ALTER TABLE alerts ADD COLUMN alert_key TEXT", [])
            .map_err(|e| e.to_string())?;
    }

    // Migration: per-day attribution. Old sessions table had PK (session_id, source) and no
    // date column — recreate with a date column + PK (session_id, source, date) so a session
    // spanning midnight stores one row per day. Existing rows copied with date=date(start_time);
    // the next full scan repopulates correct per-day (local) rows.
    let has_date: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = 'date'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if has_date == 0 {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute_batch(
            "ALTER TABLE sessions RENAME TO sessions_v1;
            CREATE TABLE sessions (
                session_id TEXT NOT NULL, source TEXT NOT NULL, date TEXT NOT NULL DEFAULT '',
                model TEXT, project TEXT, start_time TEXT, end_time TEXT,
                input_tokens INTEGER DEFAULT 0, output_tokens INTEGER DEFAULT 0,
                cache_read_tokens INTEGER DEFAULT 0, cache_creation_tokens INTEGER DEFAULT 0,
                reasoning_output_tokens INTEGER DEFAULT 0, disk_write_bytes INTEGER DEFAULT 0,
                cost REAL DEFAULT 0, scanned_at TEXT DEFAULT (datetime('now')),
                PRIMARY KEY (session_id, source, date)
            );
            INSERT INTO sessions (session_id, source, date, model, project, start_time, end_time,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                reasoning_output_tokens, disk_write_bytes, cost, scanned_at)
            SELECT session_id, source, date(start_time), model, project, start_time, end_time,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                reasoning_output_tokens, disk_write_bytes, cost, scanned_at FROM sessions_v1;
            DROP TABLE sessions_v1;
            CREATE INDEX IF NOT EXISTS idx_sessions_start ON sessions(start_time);
            CREATE INDEX IF NOT EXISTS idx_sessions_source ON sessions(source);
            CREATE INDEX IF NOT EXISTS idx_sessions_model ON sessions(model);
            CREATE INDEX IF NOT EXISTS idx_sessions_date ON sessions(date);",
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }

    // Date index — created after the per-day migration so the column is guaranteed to exist
    // (the initial execute_batch can't reference `date` on a pre-migration DB).
    conn.execute("CREATE INDEX IF NOT EXISTS idx_sessions_date ON sessions(date)", [])
        .map_err(|e| e.to_string())?;

    // Activity category 迁移: 3 个新增列,各幂等
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
        if need(&conn, col) {
            match conn.execute(ddl, []) {
                Ok(_) => {}
                Err(e) => {
                    let msg = e.to_string();
                    if !msg.contains("duplicate column") {
                        return Err(msg);
                    }
                }
            }
        }
    }

    Ok(conn)
}

fn get_db_connection() -> Result<rusqlite::Connection, String> {
    let db_path = get_costdog_db_path();
    if !db_path.exists() {
        return ensure_db_exists();
    }
    rusqlite::Connection::open(&db_path).map_err(|e| e.to_string())
}

// Decode project directory name back to a path-like project name
// Windows: "D--codes-costdog" -> "D:\codes\costdog"
// macOS/Linux: "Users-bruce-codes-costdog" -> "/Users/bruce/codes/costdog"
fn decode_project_dir(dir_name: &str) -> String {
    if cfg!(target_os = "windows") {
        // Windows: "D--codes-costdog" -> "D:\codes\costdog"
        let parts: Vec<&str> = dir_name.split("--").collect();
        if parts.len() == 2 {
            let drive = parts[0];
            let rest = parts[1].replace('-', "\\");
            format!("{}:\\{}", drive, rest)
        } else {
            dir_name.replace('-', "\\")
        }
    } else {
        // macOS/Linux: "Users-bruce-codes-costdog" -> "/Users/bruce/codes/costdog"
        format!("/{}", dir_name.replace('-', "/"))
    }
}

fn local_date(ts: &str) -> String {
    if ts.is_empty() {
        return String::new();
    }
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// Parse a single Claude Code JSONL session file into per-(session_id, date) SessionData buckets.
/// `project_display` is the human-readable project path derived from the parent directory name.
fn parse_claude_jsonl(file_path: &std::path::Path, project_display: &str) -> Vec<SessionData> {
    let file = match fs::File::open(file_path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let reader = BufReader::new(file);

    let mut session_map: HashMap<(String, String), SessionData> = HashMap::new();

    for line_result in reader.lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let data: serde_json::Value = match serde_json::from_str(&line) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let record_type = data["type"].as_str().unwrap_or("");
        let session_id = data["sessionId"].as_str().unwrap_or("").to_string();
        if session_id.is_empty() {
            continue;
        }

        let timestamp = data["timestamp"].as_str().unwrap_or("").to_string();
        let date = local_date(&timestamp);
        let key = (session_id.clone(), date.clone());

        // Initialize (session, day) bucket if not exists
        if !session_map.contains_key(&key) {
            let cwd = data["cwd"].as_str().unwrap_or("");
            let project = if !cwd.is_empty() {
                cwd.to_string()
            } else {
                project_display.to_string()
            };

            session_map.insert(key.clone(), SessionData {
                session_id: session_id.clone(),
                source: "claude-code".to_string(),
                date: date.clone(),
                model: "unknown".to_string(),
                project,
                start_time: timestamp.clone(),
                end_time: timestamp.clone(),
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                reasoning_tokens: 0,
                disk_write_bytes: 0,
                cost: 0.0,
                tool_calls: HashMap::new(),
                git_branch: None,
                activity_category: String::new(),
            });
        }

        // Update timestamps
        if let Some(entry) = session_map.get_mut(&key) {
            if !timestamp.is_empty() {
                if entry.start_time.is_empty() || timestamp < entry.start_time {
                    entry.start_time = timestamp.clone();
                }
                if timestamp > entry.end_time {
                    entry.end_time = timestamp.clone();
                }
            }
            // gitBranch: take first non-empty value
            if entry.git_branch.is_none() {
                if let Some(b) = data["gitBranch"].as_str() {
                    if !b.is_empty() {
                        entry.git_branch = Some(b.to_string());
                    }
                }
            }
        }

        // Process assistant messages with token usage + tool collection
        if record_type == "assistant" {
            let usage = &data["message"]["usage"];
            let input_tokens = usage["input_tokens"].as_u64().unwrap_or(0);
            let output_tokens = usage["output_tokens"].as_u64().unwrap_or(0);
            let cache_read = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
            let cache_creation = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);

            if input_tokens > 0 || output_tokens > 0 || cache_creation > 0 {
                if let Some(entry) = session_map.get_mut(&key) {
                    // Update model from the latest assistant message
                    if let Some(model) = data["message"]["model"].as_str() {
                        if model != "unknown" {
                            entry.model = model.to_string();
                        }
                    }

                    entry.input_tokens += input_tokens;
                    entry.output_tokens += output_tokens;
                    entry.cache_read_tokens += cache_read;
                    entry.cache_creation_tokens += cache_creation;
                }
            }

            // Collect tool_use from content blocks
            if let Some(blocks) = data["message"]["content"].as_array() {
                for b in blocks {
                    if b["type"].as_str() == Some("tool_use") {
                        if let Some(name) = b["name"].as_str() {
                            *session_map.get_mut(&key).unwrap()
                                .tool_calls.entry(name.to_string()).or_insert(0) += 1;
                        }
                    }
                }
            }

            // Collect server_tool_use counts (web_search → WebSearch, web_fetch → WebFetch)
            if let Some(entry) = session_map.get_mut(&key) {
                let stu = &usage["server_tool_use"];
                if let Some(ws) = stu["web_search_requests"].as_u64() {
                    *entry.tool_calls.entry("WebSearch".to_string()).or_insert(0) += ws;
                }
                if let Some(wf) = stu["web_fetch_requests"].as_u64() {
                    *entry.tool_calls.entry("WebFetch".to_string()).or_insert(0) += wf;
                }
            }
        }
    }

    session_map.into_values().collect()
}

fn scan_claude_sessions() -> Vec<SessionData> {
    let projects_dir = get_claude_sessions_dir();
    if !projects_dir.exists() {
        eprintln!("[CostDog] Claude projects dir not found: {:?}", projects_dir);
        return Vec::new();
    }

    let mut sessions: Vec<SessionData> = Vec::new();
    let mut file_count = 0;

    if let Ok(projects) = fs::read_dir(&projects_dir) {
        for project_entry in projects.flatten() {
            let project_path = project_entry.path();
            if !project_path.is_dir() {
                continue;
            }

            let dir_name = project_path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            let project_display = decode_project_dir(&dir_name);

            // Find all .jsonl session files in this project directory
            if let Ok(files) = fs::read_dir(&project_path) {
                for file_entry in files.flatten() {
                    let file_path = file_entry.path();
                    if !file_path.extension().map_or(false, |ext| ext == "jsonl") {
                        continue;
                    }

                    // Skip sessions-index.json and other non-session files
                    if file_path.file_name().map_or(false, |n| n == "sessions-index.json") {
                        continue;
                    }

                    file_count += 1;

                    let file_sessions = parse_claude_jsonl(&file_path, &project_display);
                    sessions.extend(file_sessions);
                }
            }
        }
    }

    eprintln!("[CostDog] Claude scan: {} files, {} sessions", file_count, sessions.len());
    sessions
}

// Codex CLI writes rollout-*.jsonl event streams (NOT flat .json). Each line is a
// JSON object with a `type` field:
//   session_meta -> payload.id (session id), payload.cwd (project), payload.timestamp,
//                   payload.model_provider
//   turn_context -> payload.model
//   event_msg    -> payload.type == "token_count" carries payload.info.total_token_usage
//                   which is CUMULATIVE (last value wins): input_tokens, output_tokens,
//                   cached_input_tokens, reasoning_output_tokens.
// Codex input_tokens is the TOTAL prompt (includes cached), so we subtract cached to get
// the non-cached portion calculate_cost expects (cache read is billed separately at 0.1x).
fn parse_codex_rollout(path: &PathBuf) -> Option<SessionData> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);

    let mut session_id = String::new();
    let mut cwd = String::new();
    let mut model = String::from("unknown");
    let mut start_time = String::new();
    let mut end_time = String::new();
    // last cumulative token_count wins
    let mut input = 0u64;
    let mut output = 0u64;
    let mut cached = 0u64;
    let mut reasoning = 0u64;
    let mut tool_calls: HashMap<String, u64> = HashMap::new();

    for line in reader.lines() {
        let line = match line { Ok(l) => l, Err(_) => continue };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) { Ok(v) => v, Err(_) => continue };

        if let Some(ts) = v["timestamp"].as_str() {
            end_time = ts.to_string();
        }

        let rtype = v["type"].as_str().unwrap_or("");
        let payload = &v["payload"];

        if rtype == "session_meta" {
            if let Some(id) = payload["id"].as_str() { session_id = id.to_string(); }
            if let Some(c) = payload["cwd"].as_str() { cwd = c.to_string(); }
            if let Some(ts) = payload["timestamp"].as_str() {
                start_time = ts.to_string();
            } else if start_time.is_empty() {
                if let Some(ts) = v["timestamp"].as_str() { start_time = ts.to_string(); }
            }
            if let Some(mp) = payload["model_provider"].as_str() {
                if !mp.is_empty() { model = mp.to_string(); }
            }
        } else if rtype == "turn_context" {
            if let Some(m) = payload["model"].as_str() {
                if !m.is_empty() { model = m.to_string(); }
            }
        } else if rtype == "event_msg" {
            if payload["type"].as_str() == Some("token_count") {
                let total = &payload["info"]["total_token_usage"];
                if !total.is_null() {
                    input = total["input_tokens"].as_u64().unwrap_or(0);
                    output = total["output_tokens"].as_u64().unwrap_or(0);
                    cached = total["cached_input_tokens"].as_u64().unwrap_or(0);
                    reasoning = total["reasoning_output_tokens"].as_u64().unwrap_or(0);
                }
            } else if payload["type"].as_str() == Some("function_call")
                   || payload["type"].as_str() == Some("tool_call") {
                if let Some(name) = payload["name"].as_str() {
                    *tool_calls.entry(name.to_string()).or_insert(0) += 1;
                }
            }
        }
    }

    if session_id.is_empty() {
        return None;
    }

    let non_cached_input = input.saturating_sub(cached);
    let project = cwd.rsplit(|c| c == '/' || c == '\\').next().unwrap_or("").to_string();
    let date = local_date(&start_time);

    Some(SessionData {
        session_id,
        source: "codex".to_string(),
        date,
        model,
        project,
        start_time,
        end_time,
        input_tokens: non_cached_input,
        output_tokens: output,
        cache_read_tokens: cached,
        cache_creation_tokens: 0,
        reasoning_tokens: reasoning,
        disk_write_bytes: 0,
        cost: 0.0,
        tool_calls,
        git_branch: None,
        activity_category: String::new(),
    })
}

fn walk_rollout_files<F: FnMut(&PathBuf)>(dir: &PathBuf, cb: &mut F) {
    let entries = match fs::read_dir(dir) { Ok(e) => e, Err(_) => return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_rollout_files(&path, cb);
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with("rollout-") && name.ends_with(".jsonl") {
                cb(&path);
            }
        }
    }
}

fn scan_codex_sessions() -> Vec<SessionData> {
    let sessions_dir = get_codex_sessions_dir();
    if !sessions_dir.exists() {
        return Vec::new();
    }

    let mut sessions = Vec::new();
    let mut on_file = |path: &PathBuf| {
        if let Some(s) = parse_codex_rollout(path) {
            sessions.push(s);
        }
    };
    walk_rollout_files(&sessions_dir, &mut on_file);
    eprintln!("[CostDog] Codex scan: {} sessions", sessions.len());
    sessions
}

// ZCode CLI stores per-request model usage in ~/.zcode/cli/db/db.sqlite.
//   session     -> id, directory (project cwd)
//   model_usage -> one row per model request: session_id, started_at (ms epoch),
//                  model_id, status, input_tokens (NON-cached, Anthropic-style),
//                  output_tokens, reasoning_tokens, cache_creation_input_tokens,
//                  cache_read_input_tokens
// Aggregated by (session_id, LOCAL date of started_at) so a session spanning midnight
// splits across days — same rule as the Claude Code parser. input_tokens excludes cache,
// so calculate_cost (which bills cache read/creation separately) is correct as-is.
fn scan_zcode_sessions() -> Vec<SessionData> {
    let db_path = get_zcode_db_path();
    let conn = match open_readonly_db(&db_path, "model_usage") {
        Some(c) => c,
        None => return Vec::new(),
    };
    // Also need the session table for project directories.
    let has_session: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='session'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if has_session == 0 {
        return Vec::new();
    }

    // Bucket key "session_id\u{0}date" -> (bucket). project dir tracked separately.
    use std::collections::HashMap;
    struct Bucket {
        start_ms: i64,
        end_ms: i64,
        model: String,
        input: u64,
        output: u64,
        reasoning: u64,
        cache_create: u64,
        cache_read: u64,
    }
    let mut buckets: HashMap<String, Bucket> = HashMap::new();
    let mut projects: HashMap<String, String> = HashMap::new();

    let sql = "SELECT m.session_id, s.directory, m.started_at, m.model_id, \
               m.input_tokens, m.output_tokens, m.reasoning_tokens, \
               m.cache_creation_input_tokens, m.cache_read_input_tokens \
               FROM model_usage m JOIN session s ON s.id = m.session_id \
               WHERE m.status IN ('completed','error','cancelled') AND m.started_at IS NOT NULL";
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[CostDog] ZCode query failed: {}", e);
            return Vec::new();
        }
    };
    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,        // session_id
            row.get::<_, Option<String>>(1)?, // directory
            row.get::<_, i64>(2)?,           // started_at
            row.get::<_, Option<String>>(3)?, // model_id
            row.get::<_, i64>(4)?,           // input
            row.get::<_, i64>(5)?,           // output
            row.get::<_, i64>(6)?,           // reasoning
            row.get::<_, i64>(7)?,           // cache_create
            row.get::<_, i64>(8)?,           // cache_read
        ))
    }) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[CostDog] ZCode query_map failed: {}", e);
            return Vec::new();
        }
    };

    for r in rows {
        let (sid, dir, started, model, input, output, reasoning, cc, cr) = match r {
            Ok(v) => v,
            Err(_) => continue,
        };
        let date = local_date_from_ms(started);
        if date.is_empty() {
            continue;
        }
        let key = format!("{}\u{0}{}", sid, date);
        projects.entry(sid.clone()).or_insert_with(|| dir.unwrap_or_default());
        let b = buckets.entry(key.clone()).or_insert(Bucket {
            start_ms: started,
            end_ms: started,
            model: model.clone().unwrap_or_default(),
            input: 0,
            output: 0,
            reasoning: 0,
            cache_create: 0,
            cache_read: 0,
        });
        if started < b.start_ms { b.start_ms = started; }
        if started > b.end_ms { b.end_ms = started; }
        if let Some(m) = &model {
            if !m.is_empty() { b.model = m.clone(); }
        }
        b.input += input as u64;
        b.output += output as u64;
        b.reasoning += reasoning as u64;
        b.cache_create += cc as u64;
        b.cache_read += cr as u64;
    }

    let mut out = Vec::new();
    for (key, b) in buckets {
        let sid = key.split('\u{0}').next().unwrap_or("").to_string();
        let dir = projects.get(&sid).cloned().unwrap_or_default();
        let project = dir.rsplit(|c| c == '/' || c == '\\').next().unwrap_or("").to_string();
        out.push(SessionData {
            session_id: sid,
            source: "zcode".to_string(),
            date: key.split('\u{0}').nth(1).unwrap_or("").to_string(),
            model: if b.model.is_empty() { "unknown".to_string() } else { b.model },
            project,
            start_time: iso_from_ms(b.start_ms),
            end_time: iso_from_ms(b.end_ms),
            input_tokens: b.input,
            output_tokens: b.output,
            cache_read_tokens: b.cache_read,
            cache_creation_tokens: b.cache_create,
            reasoning_tokens: b.reasoning,
            disk_write_bytes: 0,
            cost: 0.0,
            tool_calls: HashMap::new(),
            git_branch: None,
            activity_category: String::new(),
        });
    }
    eprintln!("[CostDog] ZCode scan: {} sessions", out.len());
    out
}

// OpenCode (v1.14+) stores everything in ~/.local/share/opencode/opencode.db.
// The session table carries pre-aggregated cost + token columns written by the app.
// Older DBs may lack some columns — detected via PRAGMA table_info and defaulted to 0.
fn scan_opencode_sessions() -> Vec<SessionData> {
    let db_path = get_opencode_db_path();
    let conn = match open_readonly_db(&db_path, "session") {
        Some(c) => c,
        None => return Vec::new(),
    };

    // Detect columns so older schemas degrade gracefully.
    let col_names: Vec<String> = {
        let mut stmt = match conn.prepare("PRAGMA table_info(session)") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map([], |row| row.get::<_, String>(1)) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(|r| r.ok()).collect()
    };
    let has = |n: &str| col_names.iter().any(|c| c == n);
    let cost_col = if has("cost") { "cost" } else { "0" };
    let ti_col = if has("tokens_input") { "tokens_input" } else { "0" };
    let to_col = if has("tokens_output") { "tokens_output" } else { "0" };
    let tr_col = if has("tokens_reasoning") { "tokens_reasoning" } else { "0" };
    let crr_col = if has("tokens_cache_read") { "tokens_cache_read" } else { "0" };
    let cw_col = if has("tokens_cache_write") { "tokens_cache_write" } else { "0" };

    let sql = format!(
        "SELECT id, directory, model, {}, {}, {}, {}, {}, {}, time_created, time_updated FROM session",
        cost_col, ti_col, to_col, tr_col, crr_col, cw_col
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[CostDog] OpenCode query failed: {}", e);
            return Vec::new();
        }
    };
    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,                  // id
            row.get::<_, Option<String>>(1)?,          // directory
            row.get::<_, Option<String>>(2)?,          // model (JSON)
            row.get::<_, Option<f64>>(3)?,             // cost
            row.get::<_, i64>(4)?,                     // tokens_input
            row.get::<_, i64>(5)?,                     // tokens_output
            row.get::<_, i64>(6)?,                     // tokens_reasoning
            row.get::<_, i64>(7)?,                     // tokens_cache_read
            row.get::<_, i64>(8)?,                     // tokens_cache_write
            row.get::<_, Option<i64>>(9)?,             // time_created
            row.get::<_, Option<i64>>(10)?,            // time_updated
        ))
    }) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[CostDog] OpenCode query_map failed: {}", e);
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    for r in rows {
        let (id, dir, model_json, cost, ti, to, tr, crr, cw, tc, tu) = match r {
            Ok(v) => v,
            Err(_) => continue,
        };
        let started = tc.or(tu).unwrap_or(0);
        let date = local_date_from_ms(started);
        // Parse model id out of the JSON column (tolerate plain string / null).
        let model = match model_json {
            Some(s) => match serde_json::from_str::<serde_json::Value>(&s) {
                Ok(v) => v
                    .get("id").and_then(|x| x.as_str()).map(|x| x.to_string())
                    .or_else(|| v.get("modelID").and_then(|x| x.as_str()).map(|x| x.to_string()))
                    .or_else(|| v.as_str().map(|x| x.to_string()))
                    .unwrap_or_default(),
                Err(_) => s,  // not JSON → treat the raw string as the model id
            },
            None => String::new(),
        };
        let project = dir
            .as_deref()
            .unwrap_or("")
            .rsplit(|c| c == '/' || c == '\\')
            .next()
            .unwrap_or("")
            .to_string();
        out.push(SessionData {
            session_id: id,
            source: "opencode".to_string(),
            date,
            model: if model.is_empty() { "unknown".to_string() } else { model },
            project,
            start_time: iso_from_ms(tc.or(tu).unwrap_or(0)),
            end_time: iso_from_ms(tu.or(tc).unwrap_or(0)),
            input_tokens: ti as u64,
            output_tokens: to as u64,
            cache_read_tokens: crr as u64,
            cache_creation_tokens: cw as u64,
            reasoning_tokens: tr as u64,
            disk_write_bytes: 0,
            // Prefer the app's own cost (provider-accurate); full_scan recomputes when 0.
            cost: cost.unwrap_or(0.0),
            tool_calls: HashMap::new(),
            git_branch: None,
            activity_category: String::new(),
        });
    }
    eprintln!("[CostDog] OpenCode scan: {} sessions", out.len());
    out
}

// ---- Pricing ----
// Shares ~/.costdog/pricing-cache.json with the TS CLI/web so both sides agree.
// Anthropic billing: input $X/M, output $Y/M, cache read 0.1x input,
// cache write (creation) 1.25x input. Reasoning tokens billed as output.
// Note: Anthropic's usage.input_tokens is already the non-cached portion
// (separate from cache_read/cache_creation), so we do NOT subtract cache here.

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PricedModel {
    #[serde(rename = "modelId")]
    model_id: String,
    #[serde(rename = "inputPricePerMToken")]
    input: f64,
    #[serde(rename = "outputPricePerMToken")]
    output: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct PricingCache {
    models: Vec<PricedModel>,
    #[serde(rename = "fetchedAt")]
    fetched_at: String,
    /// Bumped on incompatible cache changes so a stale/bad cache is ignored & refetched.
    #[serde(default)]
    version: u32,
}

const PRICING_CACHE_VERSION: u32 = 2;

fn get_pricing_cache_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let data_dir = std::env::var("COSTDOG_DATA_DIR")
        .unwrap_or_else(|_| home.join(".costdog").to_string_lossy().to_string());
    PathBuf::from(data_dir).join("pricing-cache.json")
}

fn pricing_is_stale(fetched_at: &str) -> bool {
    match chrono::DateTime::parse_from_rfc3339(fetched_at) {
        Ok(t) => chrono::Utc::now().signed_duration_since(t).num_hours() >= 24,
        Err(_) => true,
    }
}

// OpenRouter returns prices as JSON *strings* (e.g. "prompt": "0.000015"), so Value::as_f64
// (which only parses JSON numbers) returns None and we'd store 0. Handle both string and number.
fn price_as_f64(v: &serde_json::Value) -> f64 {
    match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn fetch_openrouter_pricing() -> Result<Vec<PricedModel>, String> {
    let resp = reqwest::blocking::get("https://openrouter.ai/api/v1/models")
        .map_err(|e| format!("reqwest: {}", e))?;
    let body = resp.text().map_err(|e| format!("read body: {}", e))?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| format!("parse json: {}", e))?;
    let arr = v["data"].as_array().ok_or_else(|| "no data array".to_string())?;
    let mut out = Vec::new();
    for m in arr {
        let id = match m["id"].as_str() {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        // OpenRouter prices are per-token ($/token); normalize to $ per million tokens.
        let input = price_as_f64(&m["pricing"]["prompt"]) * 1_000_000.0;
        let output = price_as_f64(&m["pricing"]["completion"]) * 1_000_000.0;
        out.push(PricedModel { model_id: id, input, output });
    }
    Ok(out)
}

/// Load pricing: fresh cache -> fetch -> stale cache -> empty (costs become 0, "unpriced").
fn load_pricing() -> Vec<PricedModel> {
    let cache_path = get_pricing_cache_path();
    if let Ok(content) = fs::read_to_string(&cache_path) {
        if let Ok(cache) = serde_json::from_str::<PricingCache>(&content) {
            if !cache.models.is_empty()
                && !pricing_is_stale(&cache.fetched_at)
                && cache.version == PRICING_CACHE_VERSION
            {
                return cache.models;
            }
        }
    }
    if let Ok(models) = fetch_openrouter_pricing() {
        if !models.is_empty() {
            let to_write = PricingCache {
                models: models.clone(),
                fetched_at: chrono::Utc::now().to_rfc3339(),
                version: PRICING_CACHE_VERSION,
            };
            if let Some(parent) = cache_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(&cache_path, serde_json::to_string(&to_write).unwrap_or_default());
            return models;
        }
    }
    if let Ok(content) = fs::read_to_string(&cache_path) {
        if let Ok(cache) = serde_json::from_str::<PricingCache>(&content) {
            return cache.models;
        }
    }
    eprintln!("[CostDog] pricing unavailable (no cache, fetch failed) — costs will be 0");
    Vec::new()
}

/// Match a model id to (input $/M, output $/M). Tiers: exact -> '/'-suffix -> contains.
/// Replace '-' between two digits with '.', e.g. "claude-opus-4-8" -> "claude-opus-4.8".
/// Claude Code logs versions with dashes; OpenRouter lists them with dots.
fn normalize_digit_dashes(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    for i in 0..bytes.len() {
        let c = bytes[i];
        if c == b'-'
            && i > 0
            && i + 1 < bytes.len()
            && bytes[i - 1].is_ascii_digit()
            && bytes[i + 1].is_ascii_digit()
        {
            out.push('.');
        } else {
            out.push(c as char);
        }
    }
    out
}

/// Last-resort prices ($/M input, $/M output) for models OpenRouter doesn't list.
fn fallback_price(model_id: &str) -> Option<(f64, f64)> {
    let lower = model_id.to_lowercase();
    const TABLE: &[(&str, f64, f64)] = &[
        ("mimo-v2.5-pro", 0.5, 2.0),
        ("glm-5.1", 1.0, 3.0),
        ("glm-5.2", 0.95, 3.0),
    ];
    for (id, pin, pout) in TABLE {
        let id_l = id.to_lowercase();
        if lower.contains(id_l.as_str()) || id_l.contains(lower.as_str()) {
            return Some((*pin, *pout));
        }
    }
    None
}

/// Match a model id to (input $/M, output $/M). Tiers:
/// exact -> '/'-suffix -> dash/dot-normalized -> contains -> fallback table.
fn find_model_price(model_id: &str, prices: &[PricedModel]) -> Option<(f64, f64)> {
    if model_id.is_empty() {
        return None;
    }
    let lower = model_id.to_lowercase();
    for m in prices {
        if m.model_id.to_lowercase() == lower {
            return Some((m.input, m.output));
        }
    }
    for m in prices {
        if m.model_id.split('/').last().map(|s| s.to_lowercase()) == Some(lower.clone()) {
            return Some((m.input, m.output));
        }
    }
    // Claude Code: "claude-opus-4-8" (dash); OpenRouter: "anthropic/claude-opus-4.8" (dot).
    let normalized = normalize_digit_dashes(&lower);
    if normalized != lower {
        for m in prices {
            let m_lower = m.model_id.to_lowercase();
            let suffix = m_lower.split('/').last().unwrap_or("").to_string();
            if m_lower == normalized || suffix == normalized {
                return Some((m.input, m.output));
            }
        }
    }
    for m in prices {
        let a = m.model_id.to_lowercase();
        if a.contains(lower.as_str()) || lower.contains(a.as_str()) {
            return Some((m.input, m.output));
        }
    }
    fallback_price(model_id)
}

fn calculate_cost(
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    reasoning_tokens: u64,
    model: &str,
    prices: &[PricedModel],
) -> f64 {
    let (pin, pout) = match find_model_price(model, prices) {
        Some(p) => p,
        None => return 0.0,
    };
    let per_m = 1_000_000.0_f64;
    (input_tokens as f64 / per_m) * pin
        + (cache_read_tokens as f64 / per_m) * (pin * 0.1)
        + (cache_creation_tokens as f64 / per_m) * (pin * 1.25)
        + (output_tokens as f64 / per_m) * pout
        + (reasoning_tokens as f64 / per_m) * pout
}

fn upsert_session(conn: &rusqlite::Connection, session: &SessionData) -> Result<(), String> {
    // tool_calls: opencode/zcode have no tool detail → NULL; others → JSON
    let tool_calls_json: Option<String> = match session.source.as_str() {
        "opencode" | "zcode" => None,
        _ => Some(serde_json::to_string(&session.tool_calls).unwrap_or_else(|_| "{}".to_string())),
    };
    conn.execute(
        "INSERT INTO sessions (session_id, source, date, model, project, start_time, end_time,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            reasoning_output_tokens, disk_write_bytes, cost, activity_category, tool_calls, git_branch)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(session_id, source, date) DO UPDATE SET
            model = excluded.model,
            end_time = excluded.end_time,
            input_tokens = excluded.input_tokens,
            output_tokens = excluded.output_tokens,
            cache_read_tokens = excluded.cache_read_tokens,
            cache_creation_tokens = excluded.cache_creation_tokens,
            reasoning_output_tokens = excluded.reasoning_output_tokens,
            disk_write_bytes = excluded.disk_write_bytes,
            cost = excluded.cost,
            activity_category = excluded.activity_category,
            tool_calls = excluded.tool_calls,
            git_branch = excluded.git_branch,
            scanned_at = datetime('now')",
        rusqlite::params![
            session.session_id, session.source, session.date, session.model, session.project,
            session.start_time, session.end_time, session.input_tokens,
            session.output_tokens, session.cache_read_tokens,
            session.cache_creation_tokens, session.reasoning_tokens,
            session.disk_write_bytes, session.cost,
            session.activity_category,
            tool_calls_json,
            session.git_branch,
        ],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

/// Insert or refresh today's alert for `key`. One row per (key, day): if today's alert
/// already exists, update its message/level (so the amount stays fresh); otherwise insert.
/// Prevents the 30s rescan from spawning thousands of duplicate rows.
fn add_alert(conn: &rusqlite::Connection, key: &str, level: &str, message: &str) -> Result<(), String> {
    let updated = conn.execute(
        "UPDATE alerts SET message = ?, level = ?, timestamp = datetime('now','localtime'), dismissed = 0
         WHERE alert_key = ? AND date(timestamp) = date('now','localtime')",
        rusqlite::params![message, level, key],
    ).map_err(|e| e.to_string())?;
    if updated == 0 {
        conn.execute(
            "INSERT INTO alerts (level, message, alert_key, timestamp)
             VALUES (?, ?, ?, datetime('now','localtime'))",
            rusqlite::params![level, message, key],
        ).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn check_alerts(conn: &rusqlite::Connection, sessions: &[SessionData]) -> Result<(), String> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let today_sessions: Vec<&SessionData> = sessions.iter()
        .filter(|s| s.start_time.starts_with(&today))
        .collect();

    // High daily cost alert
    let today_cost: f64 = today_sessions.iter().map(|s| s.cost).sum();
    if today_cost > 10.0 {
        add_alert(conn, "daily_cost", "warn", &format!("Daily cost exceeds $10: ${:.2}", today_cost))?;
    }

    // High disk write alert
    let today_disk: u64 = today_sessions.iter().map(|s| s.disk_write_bytes).sum();
    if today_disk > 100 * 1024 * 1024 {
        add_alert(conn, "disk_write", "danger", &format!("Excessive disk writes: {:.1} MB", today_disk as f64 / 1024.0 / 1024.0))?;
    }

    Ok(())
}

fn full_scan() -> Result<usize, String> {
    let conn = ensure_db_exists()?;

    let claude_sessions = scan_claude_sessions();
    let codex_sessions = scan_codex_sessions();
    let zcode_sessions = scan_zcode_sessions();
    let opencode_sessions = scan_opencode_sessions();
    let all_sessions = [claude_sessions, codex_sessions, zcode_sessions, opencode_sessions]
        .concat()
        .into_iter()
        .filter(|s| {
            // Keep rows with tokens OR a pre-computed cost (OpenCode writes its own cost).
            s.input_tokens + s.output_tokens + s.cache_read_tokens
                + s.cache_creation_tokens + s.reasoning_tokens
                > 0
                || s.cost > 0.0
        })
        .collect::<Vec<_>>();

    eprintln!("[CostDog] Scan found {} sessions total", all_sessions.len());

    let prices = load_pricing();
    let mut new_count = 0;
    for session in &all_sessions {
        let mut s = session.clone();
        s.activity_category = classify(&s.tool_calls, s.git_branch.as_deref(), &s.source).to_string();
        // OpenCode writes its own (provider-accurate) cost into the session row.
        // Trust it when present; otherwise recompute from tokens + our pricing table.
        s.cost = if s.cost > 0.0 {
            s.cost
        } else {
            calculate_cost(
                s.input_tokens,
                s.output_tokens,
                s.cache_read_tokens,
                s.cache_creation_tokens,
                s.reasoning_tokens,
                &s.model,
                &prices,
            )
        };
        upsert_session(&conn, &s)?;
        new_count += 1;
    }

    check_alerts(&conn, &all_sessions)?;

    Ok(new_count)
}

fn get_aggregate_stats(conn: &rusqlite::Connection, start: &str, end: &str) -> Result<serde_json::Value, String> {
    let mut stmt = conn.prepare(
        "SELECT
            COUNT(*) as sessions,
            COALESCE(SUM(input_tokens), 0) as input_tokens,
            COALESCE(SUM(output_tokens), 0) as output_tokens,
            COALESCE(SUM(cache_read_tokens), 0) as cache_read_tokens,
            COALESCE(SUM(disk_write_bytes), 0) as disk_write_bytes,
            COALESCE(SUM(cost), 0) as cost
        FROM sessions
        WHERE date >= ? AND date <= ?"
    ).map_err(|e| e.to_string())?;

    let result = stmt.query_row(rusqlite::params![start, end], |row| {
        Ok(serde_json::json!({
            "sessions": row.get::<_, u64>(0)?,
            "input_tokens": row.get::<_, u64>(1)?,
            "output_tokens": row.get::<_, u64>(2)?,
            "cache_read_tokens": row.get::<_, u64>(3)?,
            "disk_write_bytes": row.get::<_, u64>(4)?,
            "cost": row.get::<_, f64>(5)?,
        }))
    }).map_err(|e| e.to_string())?;

    Ok(result)
}

fn get_top_models(conn: &rusqlite::Connection, start: &str, end: &str) -> Result<Vec<TopModel>, String> {
    let mut stmt = conn.prepare(
        "SELECT
            model,
            COUNT(*) as calls,
            SUM(cost) as cost
        FROM sessions
        WHERE date >= ? AND date <= ?
        GROUP BY model
        ORDER BY cost DESC
        LIMIT 5"
    ).map_err(|e| e.to_string())?;

    let models = stmt.query_map(rusqlite::params![start, end], |row| {
        Ok(TopModel {
            model: row.get::<_, Option<String>>(0)?.unwrap_or_else(|| "unknown".to_string()),
            calls: row.get::<_, u64>(1)?,
            cost: row.get::<_, f64>(2)?,
        })
    }).map_err(|e| e.to_string())?
    .filter_map(|r| r.ok())
    .collect();

    Ok(models)
}

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

fn date_range(days: i64) -> (String, String) {
    let now = chrono::Local::now();
    let end = now.format("%Y-%m-%d").to_string();
    let start = if days == 0 {
        end.clone()
    } else {
        (now - chrono::Duration::days(days)).format("%Y-%m-%d").to_string()
    };
    (start, end)
}

#[tauri::command]
fn resize_window(app: tauri::AppHandle, width: f64, height: f64) {
    if let Some(window) = app.get_webview_window("main") {
        window.set_size(tauri::Size::Logical(tauri::LogicalSize { width, height })).ok();
    }
}

#[tauri::command]
fn get_data() -> Result<String, String> {
    let conn = get_db_connection()?;

    let (today_start, today_end) = date_range(0);
    let (week_start, week_end) = date_range(7);
    let (month_start, month_end) = date_range(30);
    let all_start = "2000-01-01".to_string();
    let all_end = "2099-12-31".to_string();

    let today_stats = get_aggregate_stats(&conn, &today_start, &today_end)?;
    let week_stats = get_aggregate_stats(&conn, &week_start, &week_end)?;
    let month_stats = get_aggregate_stats(&conn, &month_start, &month_end)?;
    let all_stats = get_aggregate_stats(&conn, &all_start, &all_end)?;

    let today_models = get_top_models(&conn, &today_start, &today_end)?;
    let week_models = get_top_models(&conn, &week_start, &week_end)?;
    let month_models = get_top_models(&conn, &month_start, &month_end)?;
    let all_models = get_top_models(&conn, &all_start, &all_end)?;

    let today_categories = get_cost_by_category(&conn, &today_start, &today_end)?;
    let week_categories = get_cost_by_category(&conn, &week_start, &week_end)?;
    let month_categories = get_cost_by_category(&conn, &month_start, &month_end)?;
    let all_categories = get_cost_by_category(&conn, &all_start, &all_end)?;

    // Get recent sessions
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
    }).map_err(|e| e.to_string())?
    .filter_map(|r| r.ok())
    .collect();

    // Get alerts
    let mut stmt = conn.prepare(
        "SELECT level, message FROM alerts WHERE dismissed = 0 AND date(timestamp) = date('now','localtime') ORDER BY timestamp DESC LIMIT 10"
    ).map_err(|e| e.to_string())?;

    let alerts: Vec<Alert> = stmt.query_map([], |row| {
        Ok(Alert {
            level: row.get(0)?,
            message: row.get(1)?,
        })
    }).map_err(|e| e.to_string())?
    .filter_map(|r| r.ok())
    .collect();

    let to_daily_summary = |stats: &serde_json::Value, models: Vec<TopModel>, by_category: Vec<CategoryBreakdown>| -> DailySummary {
        DailySummary {
            date: String::new(),
            sessions: stats["sessions"].as_u64().unwrap_or(0),
            token_usage: TokenUsage {
                input_tokens: stats["input_tokens"].as_u64().unwrap_or(0),
                output_tokens: stats["output_tokens"].as_u64().unwrap_or(0),
                cache_read_tokens: stats["cache_read_tokens"].as_u64().unwrap_or(0),
            },
            cost: stats["cost"].as_f64().unwrap_or(0.0),
            disk_write_bytes: stats["disk_write_bytes"].as_u64().unwrap_or(0),
            top_models: models,
            by_category,
        }
    };

    let data = DashboardData {
        today: to_daily_summary(&today_stats, today_models, today_categories),
        week: to_daily_summary(&week_stats, week_models, week_categories),
        month: to_daily_summary(&month_stats, month_models, month_categories),
        all_time: to_daily_summary(&all_stats, all_models, all_categories),
        recent_sessions: recent_sessions,
        alerts: alerts,
    };

    serde_json::to_string(&data).map_err(|e| e.to_string())
}

#[tauri::command]
fn scan() -> Result<String, String> {
    let count = full_scan()?;
    Ok(format!("Scanned {} sessions", count))
}

#[tauri::command]
fn close_window(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(target_os = "macos")]
        {
            window.hide().ok();
        }
        #[cfg(not(target_os = "macos"))]
        {
            window.close().ok();
        }
    }
}

// macOS menu-bar tray: the bar window has no title bar (decorations: false),
// so the close button hides it. The tray is the only way to bring it back and
// to quit the app cleanly. Built in code; no tauri.conf.json entry needed.
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let version = app.package_info().version.clone();
    let version_i = MenuItem::with_id(app, "version", format!("CostDog v{}", version), false, None::<&str>)?;
    let show_i = MenuItem::with_id(app, "show", "Show CostDog", true, None::<&str>)?;
    let update_i = MenuItem::with_id(app, "check-update", "Check for Updates…", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "Quit CostDog", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&version_i, &show_i, &update_i, &quit_i])?;

    TrayIconBuilder::with_id("main-tray")
        .tooltip(format!("CostDog v{}", version))
        .icon(app.default_window_icon().expect("default window icon missing").clone())
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "check-update" => {
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = check_for_updates(handle).await {
                        eprintln!("[CostDog] update check failed: {}", e);
                    }
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

/// Check for a newer release; if found, prompt via native dialog and, on consent,
/// download + install + relaunch. Driven by the tray "Check for Updates" item and a
/// delayed auto-check on launch.
#[tauri::command]
async fn check_for_updates(app: tauri::AppHandle) -> Result<String, String> {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
    use tauri_plugin_updater::UpdaterExt;

    let current = app.package_info().version.clone();
    let update = match app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
    {
        Ok(Some(u)) => u,
        Ok(None) => {
            let _ = app
                .dialog()
                .message(format!("CostDog {} 已是最新版本。", current))
                .title("CostDog")
                .blocking_show();
            return Ok("up-to-date".to_string());
        }
        Err(e) => {
            let _ = app
                .dialog()
                .message(format!("检查更新失败: {}", e))
                .title("CostDog")
                .blocking_show();
            return Err(e.to_string());
        }
    };

    let install = app
        .dialog()
        .message(format!("CostDog {} is available. Update now?", update.version))
        .title("CostDog")
        .buttons(MessageDialogButtons::OkCancelCustom("Update".to_string(), "Later".to_string()))
        .blocking_show();
    if !install {
        return Ok(update.version);
    }

    // Surface download/install progress in the bar so the user sees the update happening.
    let version = update.version.clone();
    let _ = app.emit("update-status", format!("downloading {}", version));
    let app_p = app.clone();
    let app_f = app.clone();
    let mut downloaded: u64 = 0;
    let mut total: u64 = 0;
    let mut last_pct: u32 = 0;
    update
        .download_and_install(
            move |chunk_len, content_len| {
                downloaded += chunk_len as u64;
                if let Some(t) = content_len {
                    total = t;
                }
                if total > 0 {
                    let pct = (downloaded * 100 / total) as u32;
                    if pct >= last_pct + 5 || pct >= 100 {
                        last_pct = pct;
                        let _ = app_p.emit("update-progress", pct);
                    }
                }
            },
            move || {
                let _ = app_f.emit("update-status", "installing");
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    app.request_restart();
    Ok(version)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![resize_window, get_data, scan, close_window, check_for_updates])
        .setup(|app| {
            let window = app.get_webview_window("main").unwrap();
            window.set_always_on_top(true).ok();
            window.set_size(tauri::Size::Logical(tauri::LogicalSize { width: 410.0, height: 36.0 })).ok();
            window.set_title("CostDog").ok();

            // System tray (restore hidden bar + quit). Failure is non-fatal: log and continue.
            if let Err(e) = build_tray(app.handle()) {
                eprintln!("[CostDog] tray init failed: {}", e);
            }

            // Initial scan (synchronous - completes before window loads)
            if let Err(e) = full_scan() {
                eprintln!("Initial scan failed: {}", e);
            }

            // Start auto-refresh timer (every 30 seconds)
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(30));
                    if let Err(e) = full_scan() {
                        eprintln!("Auto scan failed: {}", e);
                    }
                    // Emit event to frontend to refresh data
                    let _ = app_handle.emit("refresh-data", ());
                }
            });

            // Auto-check for updates a few seconds after launch (non-blocking).
            let update_handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(5));
                let h = update_handle;
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = check_for_updates(h).await {
                        eprintln!("[CostDog] update check failed: {}", e);
                    }
                });
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dashboard_json_keys() {
        let data = DashboardData {
            today: DailySummary {
                date: "2026-06-25".to_string(),
                sessions: 5,
                token_usage: TokenUsage { input_tokens: 100, output_tokens: 200, cache_read_tokens: 50 },
                cost: 1.23,
                disk_write_bytes: 1024,
                top_models: vec![TopModel { model: "test".to_string(), calls: 3, cost: 0.5 }],
                by_category: vec![],
            },
            week: DailySummary {
                date: String::new(), sessions: 0,
                token_usage: TokenUsage { input_tokens: 0, output_tokens: 0, cache_read_tokens: 0 },
                cost: 0.0, disk_write_bytes: 0, top_models: vec![],
                by_category: vec![],
            },
            month: DailySummary {
                date: String::new(), sessions: 0,
                token_usage: TokenUsage { input_tokens: 0, output_tokens: 0, cache_read_tokens: 0 },
                cost: 0.0, disk_write_bytes: 0, top_models: vec![],
                by_category: vec![],
            },
            all_time: DailySummary {
                date: String::new(), sessions: 0,
                token_usage: TokenUsage { input_tokens: 0, output_tokens: 0, cache_read_tokens: 0 },
                cost: 0.0, disk_write_bytes: 0, top_models: vec![],
                by_category: vec![],
            },
            recent_sessions: vec![],
            alerts: vec![],
        };

        let json = serde_json::to_string(&data).unwrap();
        println!("JSON output: {}", json);

        // Verify camelCase keys
        assert!(json.contains("\"tokenUsage\""), "Expected 'tokenUsage' but got: {}", json);
        assert!(json.contains("\"inputTokens\""), "Expected 'inputTokens' but got: {}", json);
        assert!(json.contains("\"outputTokens\""), "Expected 'outputTokens' but got: {}", json);
        assert!(json.contains("\"cacheReadTokens\""), "Expected 'cacheReadTokens' but got: {}", json);
        assert!(json.contains("\"diskWriteBytes\""), "Expected 'diskWriteBytes' but got: {}", json);
        assert!(json.contains("\"topModels\""), "Expected 'topModels' but got: {}", json);
        assert!(json.contains("\"allTime\""), "Expected 'allTime' but got: {}", json);
        assert!(json.contains("\"recentSessions\""), "Expected 'recentSessions' but got: {}", json);
        assert!(json.contains("\"byCategory\""), "Expected 'byCategory' but got: {}", json);
    }

    #[test]
    fn test_recent_session_activity_category_serialization() {
        let session = RecentSession {
            session_id: "abc".to_string(),
            source: "claude-code".to_string(),
            model: Some("claude-4-sonnet".to_string()),
            project: Some("myproj".to_string()),
            start_time: Some("2026-07-06T10:00:00".to_string()),
            end_time: Some("2026-07-06T10:05:00".to_string()),
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 0,
            cost: 0.05,
            disk_write_bytes: 0,
            activity_category: Some("feature".to_string()),
        };
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("\"activityCategory\":\"feature\""), "Expected activityCategory in JSON but got: {}", json);
    }

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
    fn test_ensure_db_migration_adds_activity_columns() {
        let conn = ensure_db_exists().expect("ensure_db_exists should succeed");
        for col in ["activity_category", "tool_calls", "git_branch"] {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = ?1",
                rusqlite::params![col],
                |row| row.get(0),
            ).unwrap();
            assert_eq!(count, 1, "column {} should exist after migration", col);
        }
    }

    #[test]
    fn scan_claude_collects_tools_and_branch() {
        use std::io::Write;
        // 2-line jsonl: user with gitBranch + assistant with tool_use + server_tool_use
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

        let got = parse_claude_jsonl(&f, "test-project");
        assert_eq!(got.len(), 1);
        let s = &got[0];
        assert_eq!(s.git_branch.as_deref(), Some("feature/x"));
        assert_eq!(*s.tool_calls.get("Write").unwrap_or(&0), 1);
        assert_eq!(*s.tool_calls.get("Read").unwrap_or(&0), 1);
        assert_eq!(*s.tool_calls.get("WebSearch").unwrap_or(&0), 3);

        // cleanup
        let _ = std::fs::remove_dir_all(&dir);
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
}
