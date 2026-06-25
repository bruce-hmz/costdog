use tauri::{Manager, Emitter};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::fs;
use std::io::{Read, BufRead, BufReader};
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

fn parse_json_file(path: &PathBuf) -> Result<serde_json::Value, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).map_err(|e| e.to_string())?;
    serde_json::from_str(&contents).map_err(|e| e.to_string())
}

// Decode project directory name back to a path-like project name
// e.g., "D--codes-costdog" -> "D:\codes\costdog" (on Windows)
fn decode_project_dir(dir_name: &str) -> String {
    // The directory name encodes the path: drive letter + double-dash for each separator
    // e.g., "D--codes-costdog" or "C--Users-EDY"
    let parts: Vec<&str> = dir_name.split("--").collect();
    if parts.len() == 2 {
        // Windows drive letter: "D" + "codes-costdog"
        let drive = parts[0];
        let rest = parts[1].replace('-', "\\");
        format!("{}:\\{}", drive, rest)
    } else {
        // Fallback: just replace dashes with backslashes
        dir_name.replace('-', "\\")
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

                                if input_tokens > 0 || output_tokens > 0 {
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

fn scan_codex_sessions() -> Vec<SessionData> {
    let sessions_dir = get_codex_sessions_dir();
    if !sessions_dir.exists() {
        return Vec::new();
    }

    let mut sessions = Vec::new();

    if let Ok(files) = fs::read_dir(&sessions_dir) {
        for file in files.flatten() {
            let file_path = file.path();
            if file_path.extension().map_or(false, |ext| ext == "json") {
                if let Ok(data) = parse_json_file(&file_path) {
                    if let Some(session_id) = data["session_id"].as_str() {
                        let session = SessionData {
                            session_id: session_id.to_string(),
                            source: "codex".to_string(),
                            model: data["model"].as_str().unwrap_or("unknown").to_string(),
                            project: data["project"].as_str().unwrap_or("").to_string(),
                            start_time: data["start_time"].as_str().unwrap_or("").to_string(),
                            end_time: data["end_time"].as_str().unwrap_or("").to_string(),
                            input_tokens: data["input_tokens"].as_u64().unwrap_or(0),
                            output_tokens: data["output_tokens"].as_u64().unwrap_or(0),
                            cache_read_tokens: data["cache_read_tokens"].as_u64().unwrap_or(0),
                            disk_write_bytes: data["disk_write_bytes"].as_u64().unwrap_or(0),
                            cost: 0.0,
                        };
                        sessions.push(session);
                    }
                }
            }
        }
    }

    sessions
}

fn calculate_cost(input_tokens: u64, output_tokens: u64, _cache_tokens: u64, _model: &str) -> f64 {
    // Simple cost calculation (can be enhanced with actual pricing)
    let input_cost = (input_tokens as f64) * 0.000003;
    let output_cost = (output_tokens as f64) * 0.000015;
    input_cost + output_cost
}

fn upsert_session(conn: &rusqlite::Connection, session: &SessionData) -> Result<(), String> {
    conn.execute(
        "INSERT INTO sessions (session_id, source, model, project, start_time, end_time,
            input_tokens, output_tokens, cache_read_tokens, disk_write_bytes, cost)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(session_id, source) DO UPDATE SET
            model = excluded.model,
            end_time = excluded.end_time,
            input_tokens = excluded.input_tokens,
            output_tokens = excluded.output_tokens,
            cache_read_tokens = excluded.cache_read_tokens,
            disk_write_bytes = excluded.disk_write_bytes,
            cost = excluded.cost,
            scanned_at = datetime('now')",
        rusqlite::params![
            session.session_id, session.source, session.model, session.project,
            session.start_time, session.end_time, session.input_tokens,
            session.output_tokens, session.cache_read_tokens, session.disk_write_bytes,
            session.cost
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

    let mut new_count = 0;
    for session in &all_sessions {
        let mut s = session.clone();
        s.cost = calculate_cost(
            s.input_tokens,
            s.output_tokens,
            s.cache_read_tokens,
            &s.model,
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
        window.close().ok();
    }
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
