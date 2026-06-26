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

fn ensure_db_exists() -> Result<rusqlite::Connection, String> {
    let db_path = get_costdog_db_path();
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let conn = rusqlite::Connection::open(&db_path).map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "WAL").map_err(|e| e.to_string())?;
    conn.pragma_update(None, "synchronous", "NORMAL").map_err(|e| e.to_string())?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
            session_id TEXT NOT NULL,
            source TEXT NOT NULL,
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
            PRIMARY KEY (session_id, source)
        );

        CREATE INDEX IF NOT EXISTS idx_sessions_start ON sessions(start_time);
        CREATE INDEX IF NOT EXISTS idx_sessions_source ON sessions(source);
        CREATE INDEX IF NOT EXISTS idx_sessions_model ON sessions(model);

        CREATE TABLE IF NOT EXISTS alerts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            level TEXT NOT NULL,
            message TEXT NOT NULL,
            timestamp TEXT DEFAULT (datetime('now')),
            dismissed INTEGER DEFAULT 0
        );"
    ).map_err(|e| e.to_string())?;

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

fn scan_claude_sessions() -> Vec<SessionData> {
    let projects_dir = get_claude_sessions_dir();
    if !projects_dir.exists() {
        eprintln!("[CostDog] Claude projects dir not found: {:?}", projects_dir);
        return Vec::new();
    }

    // Aggregate session data: session_id -> accumulated data
    let mut session_map: HashMap<String, SessionData> = HashMap::new();
    let mut file_count = 0;
    let mut line_count = 0;

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

                    // Parse the JSONL file line by line
                    if let Ok(file) = fs::File::open(&file_path) {
                        let reader = BufReader::new(file);
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

                            line_count += 1;
                            let record_type = data["type"].as_str().unwrap_or("");
                            let session_id = data["sessionId"].as_str().unwrap_or("").to_string();
                            if session_id.is_empty() {
                                continue;
                            }

                            let timestamp = data["timestamp"].as_str().unwrap_or("").to_string();

                            // Initialize session entry if not exists
                            if !session_map.contains_key(&session_id) {
                                let cwd = data["cwd"].as_str().unwrap_or("");
                                let project = if !cwd.is_empty() {
                                    // Use cwd as project name if available
                                    cwd.to_string()
                                } else {
                                    project_display.clone()
                                };

                                session_map.insert(session_id.clone(), SessionData {
                                    session_id: session_id.clone(),
                                    source: "claude-code".to_string(),
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
                                });
                            }

                            // Update timestamps
                            if let Some(entry) = session_map.get_mut(&session_id) {
                                if !timestamp.is_empty() {
                                    if entry.start_time.is_empty() || timestamp < entry.start_time {
                                        entry.start_time = timestamp.clone();
                                    }
                                    if timestamp > entry.end_time {
                                        entry.end_time = timestamp.clone();
                                    }
                                }
                            }

                            // Process assistant messages with token usage
                            if record_type == "assistant" {
                                let usage = &data["message"]["usage"];
                                let input_tokens = usage["input_tokens"].as_u64().unwrap_or(0);
                                let output_tokens = usage["output_tokens"].as_u64().unwrap_or(0);
                                let cache_read = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
                                let cache_creation = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);

                                if input_tokens > 0 || output_tokens > 0 || cache_creation > 0 {
                                    if let Some(entry) = session_map.get_mut(&session_id) {
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
                            }
                        }
                    }
                }
            }
        }
    }

    let sessions: Vec<SessionData> = session_map.into_values().collect();
    eprintln!("[CostDog] Claude scan: {} files, {} lines, {} sessions", file_count, line_count, sessions.len());
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
            }
        }
    }

    if session_id.is_empty() {
        return None;
    }

    let non_cached_input = input.saturating_sub(cached);
    let project = cwd.rsplit(|c| c == '/' || c == '\\').next().unwrap_or("").to_string();

    Some(SessionData {
        session_id,
        source: "codex".to_string(),
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
}

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
        // OpenRouter prices are per-token; normalize to $ per million tokens.
        let input = m["pricing"]["prompt"].as_f64().unwrap_or(0.0) * 1_000_000.0;
        let output = m["pricing"]["completion"].as_f64().unwrap_or(0.0) * 1_000_000.0;
        out.push(PricedModel { model_id: id, input, output });
    }
    Ok(out)
}

/// Load pricing: fresh cache -> fetch -> stale cache -> empty (costs become 0, "unpriced").
fn load_pricing() -> Vec<PricedModel> {
    let cache_path = get_pricing_cache_path();
    if let Ok(content) = fs::read_to_string(&cache_path) {
        if let Ok(cache) = serde_json::from_str::<PricingCache>(&content) {
            if !cache.models.is_empty() && !pricing_is_stale(&cache.fetched_at) {
                return cache.models;
            }
        }
    }
    if let Ok(models) = fetch_openrouter_pricing() {
        if !models.is_empty() {
            let to_write = PricingCache {
                models: models.clone(),
                fetched_at: chrono::Utc::now().to_rfc3339(),
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
    for m in prices {
        let a = m.model_id.to_lowercase();
        if a.contains(lower.as_str()) || lower.contains(a.as_str()) {
            return Some((m.input, m.output));
        }
    }
    None
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
    conn.execute(
        "INSERT INTO sessions (session_id, source, model, project, start_time, end_time,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            reasoning_output_tokens, disk_write_bytes, cost)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(session_id, source) DO UPDATE SET
            model = excluded.model,
            end_time = excluded.end_time,
            input_tokens = excluded.input_tokens,
            output_tokens = excluded.output_tokens,
            cache_read_tokens = excluded.cache_read_tokens,
            cache_creation_tokens = excluded.cache_creation_tokens,
            reasoning_output_tokens = excluded.reasoning_output_tokens,
            disk_write_bytes = excluded.disk_write_bytes,
            cost = excluded.cost,
            scanned_at = datetime('now')",
        rusqlite::params![
            session.session_id, session.source, session.model, session.project,
            session.start_time, session.end_time, session.input_tokens,
            session.output_tokens, session.cache_read_tokens,
            session.cache_creation_tokens, session.reasoning_tokens,
            session.disk_write_bytes, session.cost
        ],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

fn add_alert(conn: &rusqlite::Connection, level: &str, message: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO alerts (level, message) VALUES (?, ?)",
        rusqlite::params![level, message],
    ).map_err(|e| e.to_string())?;
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
        add_alert(conn, "warn", &format!("Daily cost exceeds $10: ${:.2}", today_cost))?;
    }

    // High disk write alert
    let today_disk: u64 = today_sessions.iter().map(|s| s.disk_write_bytes).sum();
    if today_disk > 100 * 1024 * 1024 {
        add_alert(conn, "danger", &format!("Excessive disk writes: {:.1} MB", today_disk as f64 / 1024.0 / 1024.0))?;
    }

    Ok(())
}

fn full_scan() -> Result<usize, String> {
    let conn = ensure_db_exists()?;

    let claude_sessions = scan_claude_sessions();
    let codex_sessions = scan_codex_sessions();
    let all_sessions = [claude_sessions, codex_sessions].concat();

    eprintln!("[CostDog] Scan found {} sessions total", all_sessions.len());

    let prices = load_pricing();
    let mut new_count = 0;
    for session in &all_sessions {
        let mut s = session.clone();
        s.cost = calculate_cost(
            s.input_tokens,
            s.output_tokens,
            s.cache_read_tokens,
            s.cache_creation_tokens,
            s.reasoning_tokens,
            &s.model,
            &prices,
        );
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
        WHERE date(start_time) >= ? AND date(start_time) <= ?"
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
        WHERE date(start_time) >= ? AND date(start_time) <= ?
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

    // Get recent sessions
    let mut stmt = conn.prepare(
        "SELECT session_id, source, model, project, start_time, end_time,
                input_tokens, output_tokens, cache_read_tokens, cost, disk_write_bytes
        FROM sessions ORDER BY start_time DESC LIMIT 20"
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
        })
    }).map_err(|e| e.to_string())?
    .filter_map(|r| r.ok())
    .collect();

    // Get alerts
    let mut stmt = conn.prepare(
        "SELECT level, message FROM alerts WHERE dismissed = 0 ORDER BY timestamp DESC LIMIT 10"
    ).map_err(|e| e.to_string())?;

    let alerts: Vec<Alert> = stmt.query_map([], |row| {
        Ok(Alert {
            level: row.get(0)?,
            message: row.get(1)?,
        })
    }).map_err(|e| e.to_string())?
    .filter_map(|r| r.ok())
    .collect();

    let to_daily_summary = |stats: &serde_json::Value, models: Vec<TopModel>| -> DailySummary {
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
        }
    };

    let data = DashboardData {
        today: to_daily_summary(&today_stats, today_models),
        week: to_daily_summary(&week_stats, week_models),
        month: to_daily_summary(&month_stats, month_models),
        all_time: to_daily_summary(&all_stats, all_models),
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
    let show_i = MenuItem::with_id(app, "show", "Show CostDog", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "Quit CostDog", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

    TrayIconBuilder::with_id("main-tray")
        .tooltip("CostDog")
        .icon(app.default_window_icon().expect("default window icon missing").clone())
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![resize_window, get_data, scan, close_window])
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

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
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
            },
            week: DailySummary {
                date: String::new(), sessions: 0,
                token_usage: TokenUsage { input_tokens: 0, output_tokens: 0, cache_read_tokens: 0 },
                cost: 0.0, disk_write_bytes: 0, top_models: vec![],
            },
            month: DailySummary {
                date: String::new(), sessions: 0,
                token_usage: TokenUsage { input_tokens: 0, output_tokens: 0, cache_read_tokens: 0 },
                cost: 0.0, disk_write_bytes: 0, top_models: vec![],
            },
            all_time: DailySummary {
                date: String::new(), sessions: 0,
                token_usage: TokenUsage { input_tokens: 0, output_tokens: 0, cache_read_tokens: 0 },
                cost: 0.0, disk_write_bytes: 0, top_models: vec![],
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
    }
}
