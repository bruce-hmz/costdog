mod analytics;
mod budget;
mod cost_ledger;
mod source_status;

use cost_ledger::{build_cost_record, ResolvedPrice};
use source_status::SourceScanMeasurement;
use tauri::{Manager, Emitter};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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
    #[serde(rename = "cacheCreationTokens")]
    cache_creation_tokens: u64,
    #[serde(rename = "reasoningOutputTokens")]
    reasoning_output_tokens: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct TopModel {
    model: String,
    sessions: u64,
    cost: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct CostQualitySummary {
    #[serde(rename = "providerCost")]
    provider_cost: f64,
    #[serde(rename = "estimatedCost")]
    estimated_cost: f64,
    #[serde(rename = "unpricedSessions")]
    unpriced_sessions: u64,
    #[serde(rename = "unpricedTokens")]
    unpriced_tokens: u64,
    #[serde(rename = "partialSessions")]
    partial_sessions: u64,
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
    #[serde(rename = "costQuality")]
    cost_quality: CostQualitySummary,
}

#[derive(Debug, Serialize, Deserialize)]
struct RecentSession {
    session_id: String,
    source: String,
    date: String,
    model: Option<String>,
    project: Option<String>,
    start_time: Option<String>,
    end_time: Option<String>,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cost: f64,
    #[serde(rename = "costBasis")]
    cost_basis: String,
    #[serde(rename = "usageCompleteness")]
    usage_completeness: String,
    #[serde(rename = "pricingMatch")]
    pricing_match: String,
    disk_write_bytes: u64,
    #[serde(rename = "activityCategory")]
    activity_category: Option<String>,
    #[serde(rename = "automaticActivityCategory")]
    automatic_activity_category: Option<String>,
    #[serde(rename = "activityCategoryOverride")]
    activity_category_override: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Alert {
    id: i64,
    level: String,
    message: String,
    #[serde(rename = "alertKey")]
    alert_key: Option<String>,
    #[serde(rename = "alertPeriod")]
    alert_period: Option<String>,
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
    provider_cost_amount: Option<f64>,
    usage_complete: bool,
    tool_calls: HashMap<String, u64>,
    git_branch: Option<String>,
    activity_category: String,
    /// First real user request, used only during the in-memory scan. Never persisted.
    #[serde(skip)]
    user_intent: String,
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

struct SourceScanOutcome {
    measurement: SourceScanMeasurement,
    sessions: Vec<SessionData>,
    fingerprints: Vec<source_status::FileFingerprint>,
    watermark: Option<i64>,
}

#[derive(Default)]
struct FileScanResult {
    sessions: Vec<SessionData>,
    fingerprints: Vec<source_status::FileFingerprint>,
    skipped_files: u64,
    malformed_lines: u64,
    /// Highest source-row timestamp read this pass; committed only if the scan succeeds.
    watermark: Option<i64>,
}

/// A row can land in the source DB after the timestamp it carries — a request that starts
/// before a scan and finishes after it. A strict watermark would skip such rows forever,
/// so every scan re-reads the last day. The work stays bounded and nothing is lost.
const INCREMENTAL_LOOKBACK_MS: i64 = 24 * 60 * 60 * 1000;

const SCAN_TICK_SECONDS: u64 = 30;
/// Ticks to skip while the bar is hidden, i.e. one scan every 5 minutes.
const HIDDEN_SCAN_TICKS: u32 = 10;

/// Mirrors the bar's visibility for the scan thread. Every show/hide goes through
/// show_bar/hide_bar, so this stays in sync without the scan thread calling a window API —
/// those dispatch to the main thread and block, which is a poor fit for a background loop.
static BAR_VISIBLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn scan_source<F>(
    source: &str,
    path: &Path,
    expected_table: Option<&str>,
    scanner: F,
) -> SourceScanOutcome
where
    F: FnOnce() -> Result<FileScanResult, String>,
{
    let started = std::time::Instant::now();
    let detected = path.exists();
    let error = if !detected {
        None
    } else if let Some(table) = expected_table {
        if open_readonly_db(&path.to_path_buf(), table).is_some() {
            None
        } else {
            Some(format!("Cannot read expected table '{table}'"))
        }
    } else {
        fs::read_dir(path)
            .map(|_| None)
            .unwrap_or_else(|error| Some(format!("Cannot read source directory: {error}")))
    };

    let scan_result = if detected && error.is_none() {
        scanner()
    } else {
        Ok(FileScanResult::default())
    };
    let (scan_result, error) = match scan_result {
        Ok(result) => (result, error),
        Err(scan_error) => (FileScanResult::default(), Some(scan_error)),
    };
    SourceScanOutcome {
        measurement: SourceScanMeasurement {
            source: source.to_string(),
            detected,
            records_found: 0,
            priced_records: 0,
            unpriced_records: 0,
            partial_records: 0,
            skipped_files: scan_result.skipped_files,
            malformed_lines: scan_result.malformed_lines,
            duration_ms: started.elapsed().as_millis() as u64,
            error,
        },
        sessions: scan_result.sessions,
        fingerprints: scan_result.fingerprints,
        watermark: scan_result.watermark,
    }
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

fn ensure_db_exists_at(db_path: &Path) -> Result<rusqlite::Connection, String> {
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let conn = rusqlite::Connection::open(db_path).map_err(|e| e.to_string())?;
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
            alert_period TEXT,
            timestamp TEXT DEFAULT (datetime('now')),
            dismissed INTEGER DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS app_settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
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
    let has_alert_period: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('alerts') WHERE name = 'alert_period'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if has_alert_period == 0 {
        conn.execute("ALTER TABLE alerts ADD COLUMN alert_period TEXT", [])
            .map_err(|e| e.to_string())?;
    }
    conn.execute_batch(
        "UPDATE alerts
         SET alert_period = date(timestamp)
         WHERE alert_key IS NOT NULL
           AND (alert_period IS NULL OR alert_period = '');
         DELETE FROM alerts
         WHERE alert_key IS NOT NULL AND alert_period IS NOT NULL
           AND id NOT IN (
             SELECT MAX(id) FROM alerts
             WHERE alert_key IS NOT NULL AND alert_period IS NOT NULL
             GROUP BY alert_key, alert_period
           );
         CREATE UNIQUE INDEX IF NOT EXISTS idx_alerts_key_period_unique
           ON alerts(alert_key, alert_period);",
    )
    .map_err(|e| e.to_string())?;

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

    // Activity category migrations: each column is added idempotently.
    let need = |conn: &rusqlite::Connection, col: &str| -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = ?1",
            rusqlite::params![col],
            |row| row.get::<_, i64>(0),
        ).unwrap_or(0) == 0
    };
    for (col, ddl) in [
        ("activity_category", "ALTER TABLE sessions ADD COLUMN activity_category TEXT"),
        ("activity_category_override", "ALTER TABLE sessions ADD COLUMN activity_category_override TEXT"),
        ("tool_calls",        "ALTER TABLE sessions ADD COLUMN tool_calls TEXT"),
        ("git_branch",        "ALTER TABLE sessions ADD COLUMN git_branch TEXT"),
        ("project_key",       "ALTER TABLE sessions ADD COLUMN project_key TEXT"),
        ("project_display",   "ALTER TABLE sessions ADD COLUMN project_display TEXT"),
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

    backfill_project_identity(&conn)?;
    cost_ledger::ensure_schema(&conn)?;
    source_status::ensure_schema(&conn)?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_sessions_date_project
           ON sessions(date, project_key);
         CREATE INDEX IF NOT EXISTS idx_sessions_date_source
           ON sessions(date, source);
         CREATE INDEX IF NOT EXISTS idx_sessions_date_model
           ON sessions(date, model);
         CREATE INDEX IF NOT EXISTS idx_sessions_date_activity
           ON sessions(
             date,
             COALESCE(
               NULLIF(activity_category_override,''),
               NULLIF(activity_category,''),
               'other'
             )
           );",
    )
    .map_err(|e| e.to_string())?;

    Ok(conn)
}

fn ensure_db_exists() -> Result<rusqlite::Connection, String> {
    ensure_db_exists_at(&get_costdog_db_path())
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

fn normalize_project_identity(source: &str, raw_project: &str) -> (String, String) {
    let raw = raw_project.trim();
    if raw.is_empty() || raw == "unknown" {
        return (
            format!("unknown:{source}"),
            "Unknown project".to_string(),
        );
    }

    let canonical = {
        let path = PathBuf::from(raw);
        if path.is_absolute() && path.exists() {
            fs::canonicalize(path)
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        } else {
            None
        }
    };
    let normalized_input = canonical.as_deref().unwrap_or(raw).replace('\\', "/");
    let bytes = normalized_input.as_bytes();
    let is_windows_absolute = bytes.len() >= 2 && bytes[1] == b':';
    let is_absolute = normalized_input.starts_with('/') || is_windows_absolute;

    if !is_absolute {
        return (
            format!("name:{source}:{raw}"),
            raw.to_string(),
        );
    }

    let mut parts: Vec<&str> = Vec::new();
    for part in normalized_input.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            parts.pop();
        } else {
            parts.push(part);
        }
    }

    if parts.is_empty() {
        return (
            format!("unknown:{source}"),
            "Unknown project".to_string(),
        );
    }

    let mut normalized = if normalized_input.starts_with('/') {
        format!("/{}", parts.join("/"))
    } else {
        parts.join("/")
    };
    if is_windows_absolute && normalized.len() >= 2 {
        let drive = normalized[0..1].to_ascii_uppercase();
        normalized.replace_range(0..1, &drive);
    }
    let display = parts.last().copied().unwrap_or("Unknown project").to_string();
    (format!("path:{normalized}"), display)
}

fn backfill_project_identity(conn: &rusqlite::Connection) -> Result<(), String> {
    let rows = {
        let mut stmt = conn
            .prepare(
                "SELECT session_id, source, date, COALESCE(project, '')
                 FROM sessions
                 WHERE project_key IS NULL OR project_key = ''
                    OR project_display IS NULL OR project_display = ''",
            )
            .map_err(|e| e.to_string())?;
        let mapped_rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
        mapped_rows.filter_map(Result::ok).collect::<Vec<_>>()
    };

    for (session_id, source, date, project) in rows {
        let (key, display) = normalize_project_identity(&source, &project);
        conn.execute(
            "UPDATE sessions SET project_key = ?1, project_display = ?2
             WHERE session_id = ?3 AND source = ?4 AND date = ?5",
            rusqlite::params![key, display, session_id, source, date],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn local_date(ts: &str) -> String {
    if ts.is_empty() {
        return String::new();
    }
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// Extract a real user-authored text message. Tool results and Codex's injected
/// workspace envelope are deliberately ignored so they cannot pollute classification.
fn user_message_text(content: &serde_json::Value) -> Option<String> {
    let text = if let Some(text) = content.as_str() {
        text.trim().to_string()
    } else {
        content.as_array()?
            .iter()
            .filter_map(|block| match block["type"].as_str() {
                Some("text") | Some("input_text") => block["text"].as_str(),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string()
    };

    if text.is_empty()
        || text.contains("<environment_context>")
        || text.contains("<recommended_plugins>")
        || text.contains("# AGENTS.md instructions")
    {
        None
    } else {
        Some(text)
    }
}

/// Parse a single Claude Code JSONL session file into per-(session_id, date) SessionData buckets.
/// `project_display` is the human-readable project path derived from the parent directory name.
fn parse_claude_jsonl_with_diagnostics(
    file_path: &std::path::Path,
    project_display: &str,
) -> Result<(Vec<SessionData>, u64), String> {
    let file = fs::File::open(file_path)
        .map_err(|error| format!("Cannot open Claude JSONL: {error}"))?;
    let reader = BufReader::new(file);

    let mut session_map: HashMap<(String, String), SessionData> = HashMap::new();
    let mut malformed_lines = 0;

    for line_result in reader.lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => {
                malformed_lines += 1;
                continue;
            }
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let data: serde_json::Value = match serde_json::from_str(&line) {
            Ok(d) => d,
            Err(_) => {
                malformed_lines += 1;
                continue;
            }
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
                    provider_cost_amount: None,
                    usage_complete: true,
                    tool_calls: HashMap::new(),
                git_branch: None,
                activity_category: String::new(),
                user_intent: String::new(),
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

            if record_type == "user" && entry.user_intent.is_empty() {
                if let Some(text) = user_message_text(&data["message"]["content"]) {
                    entry.user_intent = text;
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

    Ok((session_map.into_values().collect(), malformed_lines))
}

#[cfg(test)]
fn parse_claude_jsonl(file_path: &std::path::Path, project_display: &str) -> Vec<SessionData> {
    parse_claude_jsonl_with_diagnostics(file_path, project_display)
        .map(|(sessions, _)| sessions)
        .unwrap_or_default()
}

fn scan_claude_sessions(conn: &rusqlite::Connection) -> Result<FileScanResult, String> {
    let projects_dir = get_claude_sessions_dir();
    scan_claude_directory(conn, &projects_dir)
}

fn scan_claude_directory(
    conn: &rusqlite::Connection,
    projects_dir: &Path,
) -> Result<FileScanResult, String> {
    if !projects_dir.exists() {
        eprintln!("[CostDog] Claude projects dir not found: {:?}", projects_dir);
        return Ok(FileScanResult::default());
    }

    let mut result = FileScanResult::default();
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
                    let fingerprint =
                        source_status::fingerprint(&file_path, "claude-code")?;
                    if !source_status::file_changed(conn, &fingerprint)? {
                        result.skipped_files += 1;
                        continue;
                    }

                    let (file_sessions, malformed_lines) =
                        parse_claude_jsonl_with_diagnostics(
                            &file_path,
                            &project_display,
                        )?;
                    result.sessions.extend(file_sessions);
                    result.malformed_lines += malformed_lines;
                    result.fingerprints.push(fingerprint);
                }
            }
        }
    }

    eprintln!(
        "[CostDog] Claude scan: {} files, {} skipped, {} sessions",
        file_count,
        result.skipped_files,
        result.sessions.len()
    );
    Ok(result)
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
fn parse_codex_rollout_with_diagnostics(
    path: &PathBuf,
) -> Result<(Option<SessionData>, u64), String> {
    let file = fs::File::open(path)
        .map_err(|error| format!("Cannot open Codex JSONL: {error}"))?;
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
    let mut user_intent = String::new();
    let mut malformed_lines = 0;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => {
                malformed_lines += 1;
                continue;
            }
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                malformed_lines += 1;
                continue;
            }
        };

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
            } else if matches!(payload["type"].as_str(), Some("function_call") | Some("tool_call")) {
                let input = payload["input"].as_str()
                    .or_else(|| payload["arguments"].as_str())
                    .unwrap_or("");
                if let Some(name) = payload["name"].as_str()
                    .and_then(|name| normalize_codex_tool(name, input))
                {
                    *tool_calls.entry(name.to_string()).or_insert(0) += 1;
                }
            }
        } else if rtype == "response_item" {
            match payload["type"].as_str() {
                Some("message") if payload["role"].as_str() == Some("user") => {
                    if user_intent.is_empty() {
                        if let Some(text) = user_message_text(&payload["content"]) {
                            user_intent = text;
                        }
                    }
                }
                Some("custom_tool_call") | Some("function_call") | Some("tool_call") => {
                    let input = payload["input"].as_str()
                        .or_else(|| payload["arguments"].as_str())
                        .unwrap_or("");
                    if let Some(name) = payload["name"].as_str()
                        .and_then(|name| normalize_codex_tool(name, input))
                    {
                        *tool_calls.entry(name.to_string()).or_insert(0) += 1;
                    }
                }
                _ => {}
            }
        }
    }

    if session_id.is_empty() {
        return Ok((None, malformed_lines));
    }

    let non_cached_input = input.saturating_sub(cached);
    let project = cwd;
    let date = local_date(&start_time);

    Ok((Some(SessionData {
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
        provider_cost_amount: None,
        usage_complete: false,
        tool_calls,
        git_branch: None,
        activity_category: String::new(),
        user_intent,
    }), malformed_lines))
}

#[cfg(test)]
fn parse_codex_rollout(path: &PathBuf) -> Option<SessionData> {
    parse_codex_rollout_with_diagnostics(path)
        .ok()
        .and_then(|(session, _)| session)
}

fn normalize_codex_tool(name: &str, input: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    // Codex App wraps real tools in a generic `exec` call. Only inspect the
    // potentially large serialized input for that wrapper.
    let wrapper_input = if lower == "exec" { input } else { "" };
    if lower == "write_stdin" {
        Some("Bash")
    } else if lower.contains("apply_patch") || lower.contains("edit") || lower.contains("write")
        || wrapper_input.contains("tools.apply_patch")
    {
        Some("Edit")
    } else if lower.contains("search") || lower.contains("web") || lower.contains("browser")
        || wrapper_input.contains("tools.web__run")
    {
        Some("WebSearch")
    } else if lower.contains("agent") || lower.contains("spawn") || lower.contains("collaboration")
        || wrapper_input.contains("spawn_agent")
    {
        Some("Agent")
    } else if lower.contains("read") || lower == "open" || lower == "find"
        || wrapper_input.contains("tools.view_image")
        || wrapper_input.contains("tools.read_mcp_resource")
    {
        Some("Read")
    } else if lower == "exec" || lower.contains("exec_command") || lower.contains("shell")
        || wrapper_input.contains("tools.exec_command")
    {
        Some("Bash")
    } else {
        None
    }
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

fn scan_codex_sessions(conn: &rusqlite::Connection) -> Result<FileScanResult, String> {
    let sessions_dir = get_codex_sessions_dir();
    if !sessions_dir.exists() {
        return Ok(FileScanResult::default());
    }

    let mut result = FileScanResult::default();
    let mut scan_error = None;
    let mut on_file = |path: &PathBuf| {
        if scan_error.is_some() {
            return;
        }
        let fingerprint = match source_status::fingerprint(path, "codex") {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                scan_error = Some(error);
                return;
            }
        };
        match source_status::file_changed(conn, &fingerprint) {
            Ok(false) => {
                result.skipped_files += 1;
                return;
            }
            Ok(true) => {}
            Err(error) => {
                scan_error = Some(error);
                return;
            }
        }
        match parse_codex_rollout_with_diagnostics(path) {
            Ok((session, malformed_lines)) => {
                result.malformed_lines += malformed_lines;
                if let Some(session) = session {
                    result.sessions.push(session);
                }
            }
            Err(error) => {
                scan_error = Some(error);
                return;
            }
        }
        result.fingerprints.push(fingerprint);
    };
    walk_rollout_files(&sessions_dir, &mut on_file);
    if let Some(error) = scan_error {
        return Err(error);
    }
    eprintln!(
        "[CostDog] Codex scan: {} skipped, {} sessions",
        result.skipped_files,
        result.sessions.len()
    );
    Ok(result)
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
// Incremental: the watermark selects *sessions* with at least one recent usage row, and
// every row those sessions own is then re-aggregated. Filtering the usage rows themselves
// would emit a bucket holding only part of a session's tokens, which upsert_session would
// write over the real total. That filter happens to be safe today only because a bucket
// spans one local date and the lookback is a full day — selecting whole sessions keeps the
// invariant local instead of resting on that coincidence.
fn scan_zcode_sessions(costdog: &rusqlite::Connection) -> Result<FileScanResult, String> {
    scan_zcode_db(costdog, &get_zcode_db_path())
}

fn scan_zcode_db(
    costdog: &rusqlite::Connection,
    db_path: &PathBuf,
) -> Result<FileScanResult, String> {
    let conn = match open_readonly_db(db_path, "model_usage") {
        Some(c) => c,
        None => return Ok(FileScanResult::default()),
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
        return Ok(FileScanResult::default());
    }

    let since = source_status::load_watermark(costdog, "zcode")? - INCREMENTAL_LOOKBACK_MS;

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

    // The IN subquery runs inside ZCode's DB, so the session set never becomes a bind
    // parameter list — a first full scan would otherwise blow SQLite's variable limit.
    let sql = "SELECT m.session_id, s.directory, m.started_at, m.model_id, \
               m.input_tokens, m.output_tokens, m.reasoning_tokens, \
               m.cache_creation_input_tokens, m.cache_read_input_tokens \
               FROM model_usage m JOIN session s ON s.id = m.session_id \
               WHERE m.status IN ('completed','error','cancelled') AND m.started_at IS NOT NULL \
                 AND m.session_id IN ( \
                   SELECT session_id FROM model_usage \
                   WHERE status IN ('completed','error','cancelled') \
                     AND started_at IS NOT NULL AND started_at > ?1 \
                 )";
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[CostDog] ZCode query failed: {}", e);
            return Ok(FileScanResult::default());
        }
    };
    let rows = match stmt.query_map(rusqlite::params![since], |row| {
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
            return Ok(FileScanResult::default());
        }
    };

    let mut watermark: Option<i64> = None;
    for r in rows {
        let (sid, dir, started, model, input, output, reasoning, cc, cr) = match r {
            Ok(v) => v,
            Err(_) => continue,
        };
        watermark = Some(watermark.map_or(started, |w: i64| w.max(started)));
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
        let project = dir;
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
            provider_cost_amount: None,
            usage_complete: true,
            tool_calls: HashMap::new(),
            git_branch: None,
            activity_category: String::new(),
            user_intent: String::new(),
        });
    }
    eprintln!("[CostDog] ZCode scan: {} sessions since {}", out.len(), since);
    Ok(FileScanResult {
        sessions: out,
        watermark,
        ..FileScanResult::default()
    })
}

// OpenCode (v1.14+) stores everything in ~/.local/share/opencode/opencode.db.
// The session table carries pre-aggregated cost + token columns written by the app.
// Older DBs may lack some columns — detected via PRAGMA table_info and defaulted to 0.
// Incremental: one row IS one session here (OpenCode pre-aggregates), so filtering rows
// by their update time cannot produce a partial session the way ZCode's usage rows would.
fn scan_opencode_sessions(costdog: &rusqlite::Connection) -> Result<FileScanResult, String> {
    scan_opencode_db(costdog, &get_opencode_db_path())
}

fn scan_opencode_db(
    costdog: &rusqlite::Connection,
    db_path: &PathBuf,
) -> Result<FileScanResult, String> {
    let conn = match open_readonly_db(db_path, "session") {
        Some(c) => c,
        None => return Ok(FileScanResult::default()),
    };

    // Detect columns so older schemas degrade gracefully.
    let col_names: Vec<String> = {
        let mut stmt = match conn.prepare("PRAGMA table_info(session)") {
            Ok(s) => s,
            Err(_) => return Ok(FileScanResult::default()),
        };
        let rows = match stmt.query_map([], |row| row.get::<_, String>(1)) {
            Ok(r) => r,
            Err(_) => return Ok(FileScanResult::default()),
        };
        rows.filter_map(|r| r.ok()).collect()
    };
    let has = |n: &str| col_names.iter().any(|c| c == n);
    let has_cost = has("cost");
    let cost_col = if has_cost { "cost" } else { "NULL" };
    let ti_col = if has("tokens_input") { "tokens_input" } else { "0" };
    let to_col = if has("tokens_output") { "tokens_output" } else { "0" };
    let tr_col = if has("tokens_reasoning") { "tokens_reasoning" } else { "0" };
    let crr_col = if has("tokens_cache_read") { "tokens_cache_read" } else { "0" };
    let cw_col = if has("tokens_cache_write") { "tokens_cache_write" } else { "0" };

    let since = source_status::load_watermark(costdog, "opencode")? - INCREMENTAL_LOOKBACK_MS;
    let sql = format!(
        "SELECT id, directory, model, {}, {}, {}, {}, {}, {}, time_created, time_updated \
         FROM session WHERE COALESCE(time_updated, time_created, 0) > ?1",
        cost_col, ti_col, to_col, tr_col, crr_col, cw_col
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[CostDog] OpenCode query failed: {}", e);
            return Ok(FileScanResult::default());
        }
    };
    let rows = match stmt.query_map(rusqlite::params![since], |row| {
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
            return Ok(FileScanResult::default());
        }
    };

    let mut out = Vec::new();
    let mut watermark: Option<i64> = None;
    for r in rows {
        let (id, dir, model_json, cost, ti, to, tr, crr, cw, tc, tu) = match r {
            Ok(v) => v,
            Err(_) => continue,
        };
        let touched_at = tu.or(tc).unwrap_or(0);
        watermark = Some(watermark.map_or(touched_at, |w: i64| w.max(touched_at)));
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
        let project = dir.unwrap_or_default();
        let usage_complete = has("tokens_input")
            && has("tokens_output")
            && has("tokens_reasoning")
            && has("tokens_cache_read")
            && has("tokens_cache_write");
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
            provider_cost_amount: if has_cost { cost } else { None },
            usage_complete,
            tool_calls: HashMap::new(),
            git_branch: None,
            activity_category: String::new(),
            user_intent: String::new(),
        });
    }
    eprintln!("[CostDog] OpenCode scan: {} sessions since {}", out.len(), since);
    Ok(FileScanResult {
        sessions: out,
        watermark,
        ..FileScanResult::default()
    })
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
fn fallback_price(model_id: &str) -> Option<ResolvedPrice> {
    let lower = model_id.to_lowercase();
    let suffix = lower.split('/').last().unwrap_or("");
    const TABLE: &[(&str, f64, f64)] = &[
        ("mimo-v2.5-pro", 0.5, 2.0),
        ("glm-5.1", 1.0, 3.0),
        ("glm-5.2", 0.95, 3.0),
    ];
    for (id, pin, pout) in TABLE {
        let id_l = id.to_lowercase();
        if lower == id_l || suffix == id_l {
            return Some(ResolvedPrice {
                model_id: id.to_string(),
                match_kind: "fallback",
                input_per_m: *pin,
                output_per_m: *pout,
                cache_read_per_m: *pin * 0.1,
                cache_creation_per_m: *pin * 1.25,
            });
        }
    }
    None
}

fn resolved_openrouter_price(model: &PricedModel, match_kind: &'static str) -> ResolvedPrice {
    ResolvedPrice {
        model_id: model.model_id.clone(),
        match_kind,
        input_per_m: model.input,
        output_per_m: model.output,
        cache_read_per_m: model.input * 0.1,
        cache_creation_per_m: model.input * 1.25,
    }
}

/// Match a model id to an auditable price snapshot. Deliberately avoid fuzzy
/// `contains` matching: an unmatched model is safer than a silently wrong bill.
fn find_model_price(model_id: &str, prices: &[PricedModel]) -> Option<ResolvedPrice> {
    if model_id.is_empty() {
        return None;
    }
    let lower = model_id.to_lowercase();
    for m in prices {
        if m.model_id.to_lowercase() == lower {
            return Some(resolved_openrouter_price(m, "exact"));
        }
    }
    for m in prices {
        if m.model_id.split('/').last().map(|s| s.to_lowercase()) == Some(lower.clone()) {
            return Some(resolved_openrouter_price(m, "suffix"));
        }
    }
    // Claude Code: "claude-opus-4-8" (dash); OpenRouter: "anthropic/claude-opus-4.8" (dot).
    let normalized = normalize_digit_dashes(&lower);
    if normalized != lower {
        for m in prices {
            let m_lower = m.model_id.to_lowercase();
            let suffix = m_lower.split('/').last().unwrap_or("").to_string();
            if m_lower == normalized || suffix == normalized {
                return Some(resolved_openrouter_price(m, "normalized"));
            }
        }
    }
    fallback_price(model_id)
}

fn upsert_session(conn: &rusqlite::Connection, session: &SessionData) -> Result<(), String> {
    // tool_calls: opencode/zcode have no tool detail → NULL; others → JSON
    let tool_calls_json: Option<String> = match session.source.as_str() {
        "opencode" | "zcode" => None,
        _ => Some(serde_json::to_string(&session.tool_calls).unwrap_or_else(|_| "{}".to_string())),
    };
    let (project_key, project_display) =
        normalize_project_identity(&session.source, &session.project);
    conn.execute(
        "INSERT INTO sessions (session_id, source, date, model, project, project_key,
            project_display, start_time, end_time,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            reasoning_output_tokens, disk_write_bytes, cost, activity_category, tool_calls, git_branch)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(session_id, source, date) DO UPDATE SET
            model = excluded.model,
            project = excluded.project,
            project_key = excluded.project_key,
            project_display = excluded.project_display,
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
            project_key, project_display, session.start_time, session.end_time, session.input_tokens,
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

/// Insert or refresh one alert per logical rule period. Updating a dismissed
/// alert deliberately preserves `dismissed = 1` until the period changes.
#[allow(dead_code)] // Generic rule infrastructure; real budget rules are added in M5.
fn upsert_alert(
    conn: &rusqlite::Connection,
    key: &str,
    period: &str,
    level: &str,
    message: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO alerts (level, message, alert_key, alert_period, timestamp, dismissed)
         VALUES (?1, ?2, ?3, ?4, datetime('now','localtime'), 0)
         ON CONFLICT(alert_key, alert_period) DO UPDATE SET
           level = excluded.level,
           message = excluded.message,
           timestamp = excluded.timestamp",
        rusqlite::params![level, message, key, period],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn dismiss_alert_in_connection(
    conn: &rusqlite::Connection,
    alert_id: i64,
) -> Result<(), String> {
    let updated = conn
        .execute(
            "UPDATE alerts SET dismissed = 1 WHERE id = ?1",
            rusqlite::params![alert_id],
        )
        .map_err(|e| e.to_string())?;
    if updated == 0 {
        return Err(format!("Alert not found: {alert_id}"));
    }
    Ok(())
}

fn full_scan() -> Result<usize, String> {
    let conn = ensure_db_exists()?;

    let mut outcomes = vec![
        scan_source(
            "claude-code",
            &get_claude_sessions_dir(),
            None,
            || scan_claude_sessions(&conn),
        ),
        scan_source(
            "codex",
            &get_codex_sessions_dir(),
            None,
            || scan_codex_sessions(&conn),
        ),
        scan_source(
            "zcode",
            &get_zcode_db_path(),
            Some("model_usage"),
            || scan_zcode_sessions(&conn),
        ),
        scan_source(
            "opencode",
            &get_opencode_db_path(),
            Some("session"),
            || scan_opencode_sessions(&conn),
        ),
    ];
    let mut scanned_sessions = Vec::new();
    for outcome in &mut outcomes {
        scanned_sessions.append(&mut outcome.sessions);
    }
    let all_sessions = scanned_sessions
        .into_iter()
        .filter(|s| {
            // Keep rows with tokens OR a pre-computed cost (OpenCode writes its own cost).
            s.input_tokens + s.output_tokens + s.cache_read_tokens
                + s.cache_creation_tokens + s.reasoning_tokens
                > 0
                || s.provider_cost_amount.is_some()
        })
        .collect::<Vec<_>>();

    eprintln!("[CostDog] Scan found {} sessions total", all_sessions.len());

    let prices = load_pricing();
    let mut new_count = 0;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    for session in &all_sessions {
        let mut s = session.clone();
        s.activity_category = classify_with_intent(
            &s.tool_calls,
            s.git_branch.as_deref(),
            &s.source,
            &s.user_intent,
        ).to_string();
        let cost_record = build_cost_record(
            &s.model,
            s.input_tokens,
            s.output_tokens,
            s.cache_read_tokens,
            s.cache_creation_tokens,
            s.reasoning_tokens,
            s.provider_cost_amount,
            s.usage_complete,
            find_model_price(&s.model, &prices),
        );
        s.cost = cost_record.cost;
        upsert_session(&tx, &s)?;
        cost_ledger::upsert(
            &tx,
            &s.session_id,
            &s.source,
            &s.date,
            &cost_record,
        )?;
        new_count += 1;
    }
    tx.commit().map_err(|e| e.to_string())?;

    // Fingerprints and watermarks advance only here, after the session rows above are
    // committed — a failed scan must be retried from the same starting point, not skipped.
    let metadata_tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    for outcome in &mut outcomes {
        source_status::save_fingerprints(&metadata_tx, &outcome.fingerprints)?;
        if outcome.measurement.error.is_none() {
            if let Some(watermark) = outcome.watermark {
                source_status::save_watermark(
                    &metadata_tx,
                    &outcome.measurement.source,
                    watermark,
                )?;
            }
        }
        source_status::refresh_measurement_counts(
            &metadata_tx,
            &mut outcome.measurement,
        )?;
        source_status::save_measurement(&metadata_tx, &outcome.measurement)?;
    }
    metadata_tx.commit().map_err(|e| e.to_string())?;
    let budget_status = budget::get_status(&conn, pricing_cache_age_hours())?;
    for event in budget::threshold_events(&budget_status) {
        upsert_alert(
            &conn,
            &event.key,
            &event.period,
            &event.level,
            &event.message,
        )?;
    }

    Ok(new_count)
}

fn get_aggregate_stats(conn: &rusqlite::Connection, start: &str, end: &str) -> Result<serde_json::Value, String> {
    let unique_sessions: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM (
                SELECT DISTINCT source, session_id
                FROM sessions
                WHERE date >= ?1 AND date <= ?2
            )",
            rusqlite::params![start, end],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    let mut stmt = conn.prepare(
        "SELECT
            COALESCE(SUM(s.input_tokens), 0) as input_tokens,
            COALESCE(SUM(s.output_tokens), 0) as output_tokens,
            COALESCE(SUM(s.cache_read_tokens), 0) as cache_read_tokens,
            COALESCE(SUM(s.cache_creation_tokens), 0) as cache_creation_tokens,
            COALESCE(SUM(s.reasoning_output_tokens), 0) as reasoning_output_tokens,
            COALESCE(SUM(s.disk_write_bytes), 0) as disk_write_bytes,
            COALESCE(SUM(sc.cost), 0) as cost
        FROM sessions s
        JOIN session_costs sc
          ON sc.session_id = s.session_id AND sc.source = s.source AND sc.date = s.date
        WHERE s.date >= ? AND s.date <= ?"
    ).map_err(|e| e.to_string())?;

    let result = stmt.query_row(rusqlite::params![start, end], |row| {
        Ok(serde_json::json!({
            "sessions": unique_sessions,
            "input_tokens": row.get::<_, u64>(0)?,
            "output_tokens": row.get::<_, u64>(1)?,
            "cache_read_tokens": row.get::<_, u64>(2)?,
            "cache_creation_tokens": row.get::<_, u64>(3)?,
            "reasoning_output_tokens": row.get::<_, u64>(4)?,
            "disk_write_bytes": row.get::<_, u64>(5)?,
            "cost": row.get::<_, f64>(6)?,
        }))
    }).map_err(|e| e.to_string())?;

    Ok(result)
}

fn get_cost_quality(
    conn: &rusqlite::Connection,
    start: &str,
    end: &str,
) -> Result<CostQualitySummary, String> {
    let (provider_cost, estimated_cost, unpriced_tokens): (f64, f64, u64) = conn
        .query_row(
            "SELECT
                COALESCE(SUM(CASE WHEN sc.cost_basis = 'provider' THEN sc.cost ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN sc.cost_basis = 'estimated' THEN sc.cost ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN sc.cost_basis = 'unpriced'
                    THEN s.input_tokens + s.output_tokens + s.cache_read_tokens
                       + s.cache_creation_tokens + s.reasoning_output_tokens
                    ELSE 0 END), 0)
             FROM sessions s
             JOIN session_costs sc
               ON sc.session_id = s.session_id AND sc.source = s.source AND sc.date = s.date
             WHERE s.date >= ?1 AND s.date <= ?2",
            rusqlite::params![start, end],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| e.to_string())?;

    let count_distinct = |predicate: &str| -> Result<u64, String> {
        let sql = format!(
            "SELECT COUNT(*) FROM (
                SELECT DISTINCT s.source, s.session_id
                FROM sessions s
                JOIN session_costs sc
                  ON sc.session_id = s.session_id AND sc.source = s.source AND sc.date = s.date
                WHERE s.date >= ?1 AND s.date <= ?2 AND {predicate}
            )"
        );
        conn.query_row(&sql, rusqlite::params![start, end], |row| row.get(0))
            .map_err(|e| e.to_string())
    };

    Ok(CostQualitySummary {
        provider_cost,
        estimated_cost,
        unpriced_sessions: count_distinct("sc.cost_basis = 'unpriced'")?,
        unpriced_tokens,
        partial_sessions: count_distinct("sc.usage_completeness = 'partial'")?,
    })
}

fn get_top_models(conn: &rusqlite::Connection, start: &str, end: &str) -> Result<Vec<TopModel>, String> {
    let mut stmt = conn.prepare(
        "SELECT
            model,
            COUNT(*) as sessions,
            SUM(cost) as cost
        FROM (
            SELECT s.model, s.source, s.session_id, SUM(sc.cost) AS cost
            FROM sessions s
            JOIN session_costs sc
              ON sc.session_id = s.session_id AND sc.source = s.source AND sc.date = s.date
            WHERE s.date >= ? AND s.date <= ?
            GROUP BY s.model, s.source, s.session_id
        )
        GROUP BY model
        ORDER BY cost DESC
        LIMIT 5"
    ).map_err(|e| e.to_string())?;

    let models = stmt.query_map(rusqlite::params![start, end], |row| {
        Ok(TopModel {
            model: row.get::<_, Option<String>>(0)?.unwrap_or_else(|| "unknown".to_string()),
            sessions: row.get::<_, u64>(1)?,
            cost: row.get::<_, f64>(2)?,
        })
    }).map_err(|e| e.to_string())?
    .filter_map(|r| r.ok())
    .collect();

    Ok(models)
}

fn get_cost_by_category(conn: &rusqlite::Connection, start: &str, end: &str) -> Result<Vec<CategoryBreakdown>, String> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(NULLIF(s.activity_category_override,''), NULLIF(s.activity_category,''), 'other') AS key,
                COUNT(*) AS sessions,
                COALESCE(SUM(sc.cost), 0) AS cost,
                COALESCE(SUM(s.input_tokens + s.output_tokens + s.cache_read_tokens
                    + s.cache_creation_tokens + s.reasoning_output_tokens), 0) AS tokens
         FROM sessions s
         JOIN session_costs sc
           ON sc.session_id = s.session_id AND sc.source = s.source AND sc.date = s.date
         WHERE s.date >= ? AND s.date <= ?
         GROUP BY COALESCE(NULLIF(s.activity_category_override,''), NULLIF(s.activity_category,''), 'other')
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
        (now - chrono::Duration::days(days.saturating_sub(1)))
            .format("%Y-%m-%d")
            .to_string()
    };
    (start, end)
}

const ACTIVITY_CATEGORIES: [&str; 9] = [
    "feature", "bugfix", "refactor", "docs", "research", "debug", "agent", "explore", "other",
];

fn set_activity_category_override_in_connection(
    conn: &rusqlite::Connection,
    session_id: &str,
    source: &str,
    date: &str,
    category: Option<&str>,
) -> Result<(), String> {
    let category = category.map(str::trim).filter(|value| !value.is_empty());
    if let Some(value) = category {
        if !ACTIVITY_CATEGORIES.contains(&value) {
            return Err(format!(
                "Invalid activity category '{value}'. Expected one of: {}",
                ACTIVITY_CATEGORIES.join(", ")
            ));
        }
    }

    let changed = conn.execute(
        "UPDATE sessions SET activity_category_override = ?1
         WHERE session_id = ?2 AND source = ?3 AND date = ?4",
        rusqlite::params![category, session_id, source, date],
    ).map_err(|e| format!("Failed to update activity category: {e}"))?;

    if changed == 0 {
        return Err(format!(
            "Session not found for session_id='{session_id}', source='{source}', date='{date}'"
        ));
    }

    Ok(())
}

#[tauri::command]
fn set_activity_category_override(
    session_id: String,
    source: String,
    date: String,
    category: Option<String>,
) -> Result<(), String> {
    let conn = get_db_connection()?;
    set_activity_category_override_in_connection(
        &conn, &session_id, &source, &date, category.as_deref(),
    )
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

    let today_quality = get_cost_quality(&conn, &today_start, &today_end)?;
    let week_quality = get_cost_quality(&conn, &week_start, &week_end)?;
    let month_quality = get_cost_quality(&conn, &month_start, &month_end)?;
    let all_quality = get_cost_quality(&conn, &all_start, &all_end)?;

    // Get recent sessions
    let mut stmt = conn.prepare(
        "SELECT s.session_id, s.source, s.date, s.model,
                COALESCE(NULLIF(s.project_display,''), NULLIF(s.project,''), 'Unknown project'),
                s.start_time, s.end_time, s.input_tokens, s.output_tokens,
                s.cache_read_tokens, sc.cost, s.disk_write_bytes,
                COALESCE(NULLIF(s.activity_category_override,''), NULLIF(s.activity_category,''), 'other'),
                COALESCE(NULLIF(s.activity_category,''), 'other'),
                NULLIF(s.activity_category_override,''),
                sc.cost_basis, sc.usage_completeness, sc.pricing_match
        FROM sessions s
        JOIN session_costs sc
          ON sc.session_id = s.session_id AND sc.source = s.source AND sc.date = s.date
        ORDER BY s.date DESC, s.start_time DESC LIMIT 20"
    ).map_err(|e| e.to_string())?;

    let recent_sessions: Vec<RecentSession> = stmt.query_map([], |row| {
        Ok(RecentSession {
            session_id: row.get(0)?,
            source: row.get(1)?,
            date: row.get(2)?,
            model: row.get(3)?,
            project: row.get(4)?,
            start_time: row.get(5)?,
            end_time: row.get(6)?,
            input_tokens: row.get(7)?,
            output_tokens: row.get(8)?,
            cache_read_tokens: row.get(9)?,
            cost: row.get(10)?,
            cost_basis: row.get(15)?,
            usage_completeness: row.get(16)?,
            pricing_match: row.get(17)?,
            disk_write_bytes: row.get(11)?,
            activity_category: row.get::<_, Option<String>>(12)?,
            automatic_activity_category: row.get::<_, Option<String>>(13)?,
            activity_category_override: row.get::<_, Option<String>>(14)?,
        })
    }).map_err(|e| e.to_string())?
    .filter_map(|r| r.ok())
    .collect();

    // Get alerts
    let mut stmt = conn.prepare(
        "SELECT id, level, message, alert_key, alert_period
         FROM alerts
         WHERE dismissed = 0 AND (
           (length(alert_period) = 10 AND alert_period = date('now','localtime'))
           OR
           (length(alert_period) = 7 AND alert_period = strftime('%Y-%m','now','localtime'))
         )
         ORDER BY timestamp DESC
         LIMIT 10"
    ).map_err(|e| e.to_string())?;

    let alerts: Vec<Alert> = stmt.query_map([], |row| {
        Ok(Alert {
            id: row.get(0)?,
            level: row.get(1)?,
            message: row.get(2)?,
            alert_key: row.get(3)?,
            alert_period: row.get(4)?,
        })
    }).map_err(|e| e.to_string())?
    .filter_map(|r| r.ok())
    .collect();

    let to_daily_summary = |stats: &serde_json::Value,
                            models: Vec<TopModel>,
                            by_category: Vec<CategoryBreakdown>,
                            cost_quality: CostQualitySummary|
     -> DailySummary {
        DailySummary {
            date: String::new(),
            sessions: stats["sessions"].as_u64().unwrap_or(0),
            token_usage: TokenUsage {
                input_tokens: stats["input_tokens"].as_u64().unwrap_or(0),
                output_tokens: stats["output_tokens"].as_u64().unwrap_or(0),
                cache_read_tokens: stats["cache_read_tokens"].as_u64().unwrap_or(0),
                cache_creation_tokens: stats["cache_creation_tokens"].as_u64().unwrap_or(0),
                reasoning_output_tokens: stats["reasoning_output_tokens"].as_u64().unwrap_or(0),
            },
            cost: stats["cost"].as_f64().unwrap_or(0.0),
            disk_write_bytes: stats["disk_write_bytes"].as_u64().unwrap_or(0),
            top_models: models,
            by_category,
            cost_quality,
        }
    };

    let data = DashboardData {
        today: to_daily_summary(&today_stats, today_models, today_categories, today_quality),
        week: to_daily_summary(&week_stats, week_models, week_categories, week_quality),
        month: to_daily_summary(&month_stats, month_models, month_categories, month_quality),
        all_time: to_daily_summary(&all_stats, all_models, all_categories, all_quality),
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
fn dismiss_alert(alert_id: i64) -> Result<(), String> {
    let conn = ensure_db_exists()?;
    dismiss_alert_in_connection(&conn, alert_id)
}

#[tauri::command]
fn get_source_status() -> Result<Vec<source_status::SourceStatus>, String> {
    let conn = ensure_db_exists()?;
    source_status::load_statuses(
        &conn,
        &[
            ("claude-code", get_claude_sessions_dir()),
            ("codex", get_codex_sessions_dir()),
            ("zcode", get_zcode_db_path()),
            ("opencode", get_opencode_db_path()),
        ],
    )
}

#[tauri::command]
fn get_analytics(
    range: String,
    filters: Option<analytics::AnalyticsFilters>,
) -> Result<analytics::AnalyticsResponse, String> {
    let conn = ensure_db_exists()?;
    analytics::get_analytics(&conn, &range, filters.unwrap_or_default())
}

fn pricing_cache_age_hours() -> Option<i64> {
    let content = fs::read_to_string(get_pricing_cache_path()).ok()?;
    let cache = serde_json::from_str::<PricingCache>(&content).ok()?;
    let fetched_at = chrono::DateTime::parse_from_rfc3339(&cache.fetched_at).ok()?;
    Some(
        chrono::Utc::now()
            .signed_duration_since(fetched_at.with_timezone(&chrono::Utc))
            .num_hours()
            .max(0),
    )
}

#[tauri::command]
fn get_monthly_budget() -> Result<budget::BudgetStatus, String> {
    let conn = ensure_db_exists()?;
    budget::get_status(&conn, pricing_cache_age_hours())
}

#[tauri::command]
fn set_monthly_budget(amount_usd: Option<f64>) -> Result<budget::BudgetStatus, String> {
    let conn = ensure_db_exists()?;
    budget::set_budget(&conn, amount_usd)?;
    let status = budget::get_status(&conn, pricing_cache_age_hours())?;
    for event in budget::threshold_events(&status) {
        upsert_alert(
            &conn,
            &event.key,
            &event.period,
            &event.level,
            &event.message,
        )?;
    }
    Ok(status)
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
        .invoke_handler(tauri::generate_handler![resize_window, get_data, get_analytics, get_source_status, get_monthly_budget, set_monthly_budget, set_activity_category_override, dismiss_alert, scan, close_window, check_for_updates])
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

            // Auto-refresh: every 30s while the bar is on screen, every 10th tick (5 min)
            // while it is hidden — nobody is reading the numbers then. Ticking at a fixed
            // 30s rather than sleeping longer keeps the delay after the bar reappears
            // bounded by one tick.
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut ticks_since_scan = 0;
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(SCAN_TICK_SECONDS));
                    ticks_since_scan += 1;
                    let visible = BAR_VISIBLE.load(std::sync::atomic::Ordering::Relaxed);
                    let ticks_needed = if visible { 1 } else { HIDDEN_SCAN_TICKS };
                    if ticks_since_scan < ticks_needed {
                        continue;
                    }
                    ticks_since_scan = 0;
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

fn contains_intent_keyword(text: &str, keyword: &str) -> bool {
    if !keyword.is_ascii() {
        return text.contains(keyword);
    }

    let mut start = 0;
    while let Some(offset) = text[start..].find(keyword) {
        let index = start + offset;
        let before_is_word = text[..index]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        let after = index + keyword.len();
        let after_is_word = text[after..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        if !before_is_word && !after_is_word {
            return true;
        }
        start = index + keyword.len();
    }
    false
}

fn intent_category(user_intent: &str) -> Option<&'static str> {
    let text = user_intent.to_lowercase();
    let has_any = |keywords: &[&str]| keywords.iter().any(|word| contains_intent_keyword(&text, word));
    let fix_is_negated = has_any(&["不要修复", "无需修复", "不用修复", "do not fix", "don't fix"]);
    let support_is_question = has_any(&["支持什么", "是否支持", "能否支持", "支不支持", "what does", "does it support"]);

    // The requested outcome is more reliable than incidental tools used to achieve it.
    if has_any(&["重构", "整理代码", "清理代码", "refactor", "cleanup"]) {
        Some("refactor")
    } else if has_any(&["文档", "说明书", "readme", "documentation", "docs"]) {
        Some("docs")
    } else if !fix_is_negated && has_any(&["修复", "修一下", "不准确", "不正确", "不准", "有问题", "不能用", "fix bug", "fix the bug", "fix", "broken", "regression"]) {
        Some("bugfix")
    } else if has_any(&["排查", "定位原因", "调试", "报错", "崩溃", "根因", "debug", "investigate", "root cause"]) {
        Some("debug")
    } else if has_any(&["子代理", "多代理", "并行代理", "subagent", "sub-agent", "multi-agent"]) {
        Some("agent")
    } else if has_any(&["阅读代码", "审查代码", "解释代码", "代码分析", "code review", "explain the code", "read the code"]) {
        Some("explore")
    } else if has_any(&["调研", "研究如何", "查找资料", "推荐", "对比", "分析", "research", "look up", "compare"]) {
        Some("research")
    } else if has_any(&["添加", "新增", "实现", "开发", "创建", "接入", "add", "implement", "create", "build"])
        || (!support_is_question && has_any(&["支持", "support"]))
    {
        Some("feature")
    } else if has_any(&["研究", "搜索", "search"]) {
        Some("research")
    } else {
        None
    }
}

fn classify_with_intent(
    tool_calls: &HashMap<String, u64>,
    git_branch: Option<&str>,
    source: &str,
    user_intent: &str,
) -> &'static str {
    if let Some(category) = intent_category(user_intent) {
        return category;
    }
    classify(tool_calls, git_branch, source)
}

/// 判定一个 session 的活动类型。规则链首个命中即返回(spec §5)。
fn classify(tool_calls: &HashMap<String, u64>, git_branch: Option<&str>, source: &str) -> &'static str {
    let has_tool_detail = matches!(source, "claude-code" | "codex");

    // ① 无用户意图时，gitBranch 首段精确匹配是最可信的兜底信号。
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
    if total == 0 { return "other"; }
    if ratio("Task") + ratio("Agent") > 0.40 { return "agent"; }
    if ratio("WebSearch") + ratio("WebFetch") > 0.40 { return "research"; }
    let write_edit = ratio("Write") + ratio("Edit");
    if (ratio("Read") + ratio("Grep") + ratio("Glob") > 0.60) && write_edit <= 0.20 { return "explore"; }

    // ③ 兜底
    "other"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "costdog_{}_{}_{}.sqlite",
            name,
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }

    fn write_zcode_fixture(path: &PathBuf, rows: &[(&str, i64, u64)]) {
        let source = rusqlite::Connection::open(path).unwrap();
        source
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS session (id TEXT PRIMARY KEY, directory TEXT);
                 CREATE TABLE IF NOT EXISTS model_usage (
                   session_id TEXT, started_at INTEGER, model_id TEXT, status TEXT,
                   input_tokens INTEGER, output_tokens INTEGER, reasoning_tokens INTEGER,
                   cache_creation_input_tokens INTEGER, cache_read_input_tokens INTEGER);",
            )
            .unwrap();
        for (session_id, started_at, input) in rows {
            source
                .execute(
                    "INSERT OR IGNORE INTO session(id, directory) VALUES (?1, '/tmp/proj')",
                    rusqlite::params![session_id],
                )
                .unwrap();
            source
                .execute(
                    "INSERT INTO model_usage(session_id, started_at, model_id, status,
                       input_tokens, output_tokens, reasoning_tokens,
                       cache_creation_input_tokens, cache_read_input_tokens)
                     VALUES (?1, ?2, 'glm-4', 'completed', ?3, 0, 0, 0, 0)",
                    rusqlite::params![session_id, started_at, *input as i64],
                )
                .unwrap();
        }
    }

    // Pins both halves of the incremental contract: history outside the lookback window
    // stops being re-read, and a session that gains a row still comes back with its full
    // token total rather than just the increment.
    #[test]
    fn zcode_rescan_reaggregates_whole_sessions_not_just_new_rows() {
        let costdog = rusqlite::Connection::open_in_memory().unwrap();
        source_status::ensure_schema(&costdog).unwrap();
        let db_path = temp_db_path("zcode_incremental");
        let now = chrono::Utc::now().timestamp_millis();
        let ten_days_ago = now - 10 * 24 * 60 * 60 * 1000;

        write_zcode_fixture(
            &db_path,
            &[
                ("old", ten_days_ago, 500),
                ("live", now - 2 * 60 * 60 * 1000, 100),
                ("live", now - 60 * 60 * 1000, 30),
            ],
        );

        let first = scan_zcode_db(&costdog, &db_path).unwrap();
        let live_first: u64 = first
            .sessions
            .iter()
            .filter(|s| s.session_id == "live")
            .map(|s| s.input_tokens)
            .sum();
        assert_eq!(live_first, 130, "first scan reads the full history");
        assert!(first.sessions.iter().any(|s| s.session_id == "old"));
        source_status::save_watermark(&costdog, "zcode", first.watermark.unwrap()).unwrap();

        // A new usage row lands on the same session, on the same local date.
        write_zcode_fixture(&db_path, &[("live", now - 60 * 1000, 7)]);

        let second = scan_zcode_db(&costdog, &db_path).unwrap();
        let live_second: u64 = second
            .sessions
            .iter()
            .filter(|s| s.session_id == "live")
            .map(|s| s.input_tokens)
            .sum();
        assert_eq!(
            live_second, 137,
            "rescan must re-sum all rows of a touched session, not only the new one"
        );
        assert!(
            !second.sessions.iter().any(|s| s.session_id == "old"),
            "sessions untouched since the watermark must not be re-read"
        );

        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn opencode_rescan_skips_sessions_untouched_since_the_watermark() {
        let costdog = rusqlite::Connection::open_in_memory().unwrap();
        source_status::ensure_schema(&costdog).unwrap();
        let db_path = temp_db_path("opencode_incremental");
        let now = chrono::Utc::now().timestamp_millis();
        let ten_days_ago = now - 10 * 24 * 60 * 60 * 1000;

        let source = rusqlite::Connection::open(&db_path).unwrap();
        source
            .execute_batch(
                "CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT, model TEXT,
                   cost REAL, tokens_input INTEGER, tokens_output INTEGER,
                   tokens_reasoning INTEGER, tokens_cache_read INTEGER,
                   tokens_cache_write INTEGER, time_created INTEGER, time_updated INTEGER);",
            )
            .unwrap();
        let insert = |id: &str, created: i64, updated: i64, input: i64| {
            source
                .execute(
                    "INSERT INTO session VALUES (?1,'/tmp/proj','{\"id\":\"gpt\"}',0.5,?2,0,0,0,0,?3,?4)",
                    rusqlite::params![id, input, created, updated],
                )
                .unwrap();
        };
        insert("old", ten_days_ago, ten_days_ago, 500);
        insert("live", now - 60 * 60 * 1000, now - 60 * 60 * 1000, 100);

        let first = scan_opencode_db(&costdog, &db_path).unwrap();
        assert_eq!(first.sessions.len(), 2, "first scan reads the full history");
        source_status::save_watermark(&costdog, "opencode", first.watermark.unwrap()).unwrap();

        let second = scan_opencode_db(&costdog, &db_path).unwrap();
        assert_eq!(
            second.sessions.iter().map(|s| s.session_id.as_str()).collect::<Vec<_>>(),
            vec!["live"],
            "only rows updated inside the lookback window are re-read"
        );

        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn test_dashboard_json_keys() {
        let empty_quality = || CostQualitySummary {
            provider_cost: 0.0,
            estimated_cost: 0.0,
            unpriced_sessions: 0,
            unpriced_tokens: 0,
            partial_sessions: 0,
        };
        let data = DashboardData {
            today: DailySummary {
                date: "2026-06-25".to_string(),
                sessions: 5,
                token_usage: TokenUsage {
                    input_tokens: 100,
                    output_tokens: 200,
                    cache_read_tokens: 50,
                    cache_creation_tokens: 25,
                    reasoning_output_tokens: 10,
                },
                cost: 1.23,
                disk_write_bytes: 1024,
                top_models: vec![TopModel { model: "test".to_string(), sessions: 3, cost: 0.5 }],
                by_category: vec![],
                cost_quality: empty_quality(),
            },
            week: DailySummary {
                date: String::new(), sessions: 0,
                token_usage: TokenUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    cache_creation_tokens: 0,
                    reasoning_output_tokens: 0,
                },
                cost: 0.0, disk_write_bytes: 0, top_models: vec![],
                by_category: vec![],
                cost_quality: empty_quality(),
            },
            month: DailySummary {
                date: String::new(), sessions: 0,
                token_usage: TokenUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    cache_creation_tokens: 0,
                    reasoning_output_tokens: 0,
                },
                cost: 0.0, disk_write_bytes: 0, top_models: vec![],
                by_category: vec![],
                cost_quality: empty_quality(),
            },
            all_time: DailySummary {
                date: String::new(), sessions: 0,
                token_usage: TokenUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    cache_creation_tokens: 0,
                    reasoning_output_tokens: 0,
                },
                cost: 0.0, disk_write_bytes: 0, top_models: vec![],
                by_category: vec![],
                cost_quality: empty_quality(),
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
        assert!(json.contains("\"cacheCreationTokens\""), "Expected 'cacheCreationTokens' but got: {}", json);
        assert!(json.contains("\"reasoningOutputTokens\""), "Expected 'reasoningOutputTokens' but got: {}", json);
        assert!(json.contains("\"diskWriteBytes\""), "Expected 'diskWriteBytes' but got: {}", json);
        assert!(json.contains("\"topModels\""), "Expected 'topModels' but got: {}", json);
        assert!(json.contains("\"costQuality\""), "Expected 'costQuality' but got: {}", json);
        assert!(json.contains("\"allTime\""), "Expected 'allTime' but got: {}", json);
        assert!(json.contains("\"recentSessions\""), "Expected 'recentSessions' but got: {}", json);
        assert!(json.contains("\"byCategory\""), "Expected 'byCategory' but got: {}", json);
    }

    #[test]
    fn test_recent_session_activity_category_serialization() {
        let session = RecentSession {
            session_id: "abc".to_string(),
            source: "claude-code".to_string(),
            date: "2026-07-06".to_string(),
            model: Some("claude-4-sonnet".to_string()),
            project: Some("myproj".to_string()),
            start_time: Some("2026-07-06T10:00:00".to_string()),
            end_time: Some("2026-07-06T10:05:00".to_string()),
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 0,
            cost: 0.05,
            cost_basis: "estimated".to_string(),
            usage_completeness: "complete".to_string(),
            pricing_match: "exact".to_string(),
            disk_write_bytes: 0,
            activity_category: Some("feature".to_string()),
            automatic_activity_category: Some("feature".to_string()),
            activity_category_override: None,
        };
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("\"activityCategory\":\"feature\""), "Expected activityCategory in JSON but got: {}", json);
    }

    fn tc(pairs: &[(&str, u64)]) -> HashMap<String, u64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn extracts_only_real_user_message_text() {
        let cases = [
            (serde_json::json!("  plain request  "), Some("plain request")),
            (
                serde_json::json!([
                    {"type":"input_text","text":"first"},
                    {"type":"tool_result","text":"ignored"},
                    {"type":"text","text":"second"}
                ]),
                Some("first\nsecond"),
            ),
            (serde_json::json!({"text":"not an array"}), None),
            (serde_json::json!("   "), None),
            (serde_json::json!("<environment_context>injected"), None),
            (serde_json::json!("<recommended_plugins>injected"), None),
            (serde_json::json!("# AGENTS.md instructions for /tmp"), None),
        ];

        for (content, expected) in cases {
            assert_eq!(user_message_text(&content).as_deref(), expected);
        }
    }

    #[test]
    fn intent_categories_and_ascii_word_boundaries() {
        let cases = [
            ("please fix this", Some("bugfix")),
            ("refactor this module", Some("refactor")),
            ("update the docs", Some("docs")),
            ("debug the failure", Some("debug")),
            ("use a multi-agent workflow", Some("agent")),
            ("perform a code review", Some("explore")),
            ("implement support for csv", Some("feature")),
            ("research and compare options", Some("research")),
            ("分析 bug 根因", Some("debug")),
            ("perform a code review for bugs", Some("explore")),
            ("不要修复，只分析为什么不准确", Some("research")),
            ("研究如何实现导出功能", Some("research")),
            ("这个平台支持什么？", None),
            ("修复 README 中的错字", Some("docs")),
            ("ordinary conversation", None),
            ("prefixfixsuffix", None),
            ("prefixfixsuffix then fix it", Some("bugfix")),
        ];

        for (intent, expected) in cases {
            assert_eq!(intent_category(intent), expected, "intent: {intent}");
        }
    }

    #[test]
    fn normalizes_codex_tool_names_and_embedded_calls() {
        let cases = [
            ("apply_patch", "", Some("Edit")),
            ("web_search", "", Some("WebSearch")),
            ("spawn_agent", "", Some("Agent")),
            ("read_mcp_resource", "", Some("Read")),
            ("exec_command", "", Some("Bash")),
            ("exec", "await tools.apply_patch(...) ", Some("Edit")),
            ("exec", "await tools.web__run(...) ", Some("WebSearch")),
            ("exec", "await spawn_agent(...) ", Some("Agent")),
            ("exec", "await tools.view_image(...) ", Some("Read")),
            ("exec", "await tools.exec_command(...) ", Some("Bash")),
            ("write_stdin", "", Some("Bash")),
            ("unrelated_tool", "{}", None),
        ];

        for (name, input, expected) in cases {
            assert_eq!(normalize_codex_tool(name, input), expected, "tool: {name}");
        }
    }

    #[test]
    fn classify_branch_signals() {
        // gitBranch 首段精确匹配,优先于工具占比
        assert_eq!(classify(&tc(&[("Write", 10)]), Some("origin/Fix/login"), "claude-code"), "bugfix");
        assert_eq!(classify(&tc(&[("Write", 10)]), Some("refs/heads/docs/readme"), "claude-code"), "docs");
        assert_eq!(classify(&tc(&[("Edit", 10)]), Some("refactor/api"), "claude-code"), "refactor");
        assert_eq!(classify(&tc(&[("Bash", 10)]), Some("feature/donut"), "claude-code"), "feature");
        // 无斜杠 / 不在集合 → 不命中分支规则
        assert_eq!(classify(&tc(&[("Bash", 6)]), Some("feat-category"), "claude-code"), "other");
        assert_eq!(classify(&tc(&[("Write", 10)]), Some("main"), "claude-code"), "other");
    }

    #[test]
    fn classify_source_other_for_no_detail() {
        // opencode/zcode 无工具细节 → other(即使 tool_calls 为空也不算 research)
        assert_eq!(classify(&HashMap::new(), None, "opencode"), "other");
        assert_eq!(classify(&HashMap::new(), None, "zcode"), "other");
        // 没有工具信号不代表做了调研，普通问答也可能完全不调用工具。
        assert_eq!(classify(&HashMap::new(), None, "claude-code"), "other");
        assert_eq!(classify(&HashMap::new(), None, "codex"), "other");
    }

    #[test]
    fn classify_prefers_explicit_user_intent() {
        assert_eq!(
            classify_with_intent(
                &tc(&[("Bash", 8), ("Read", 2)]),
                None,
                "codex",
                "之前加了统计功能，实际使用发现分类并不准，请修复",
            ),
            "bugfix",
        );
        assert_eq!(
            classify_with_intent(
                &tc(&[("Bash", 8), ("Read", 2)]),
                None,
                "claude-code",
                "实现一个自动扫描 OpenCode 日志的新功能",
            ),
            "feature",
        );
        assert_eq!(
            classify_with_intent(
                &HashMap::new(),
                Some("docs/readme"),
                "claude-code",
                "修复登录失败的问题",
            ),
            "bugfix",
        );
    }

    #[test]
    fn parse_codex_collects_response_item_tools_and_user_intent() {
        use std::io::Write;

        let records = [
            serde_json::json!({
                "timestamp":"2026-07-15T08:00:00Z","type":"session_meta",
                "payload":{"id":"cx1","cwd":"/tmp/costdog","timestamp":"2026-07-15T08:00:00Z"}
            }),
            serde_json::json!({
                "timestamp":"2026-07-15T08:00:01Z","type":"response_item",
                "payload":{"type":"message","role":"user","content":[
                    {"type":"input_text","text":"修复分类不准确的问题"}
                ]}
            }),
            serde_json::json!({
                "timestamp":"2026-07-15T08:00:02Z","type":"response_item",
                "payload":{"type":"custom_tool_call","name":"exec","input":"..."}
            }),
            serde_json::json!({
                "timestamp":"2026-07-15T08:00:03Z","type":"response_item",
                "payload":{"type":"function_call","name":"apply_patch","arguments":"..."}
            }),
            serde_json::json!({
                "timestamp":"2026-07-15T08:00:03Z","type":"event_msg",
                "payload":{"type":"tool_call","name":"web_search","arguments":"{}"}
            }),
            serde_json::json!({
                "timestamp":"2026-07-15T08:00:04Z","type":"event_msg",
                "payload":{"type":"token_count","info":{"total_token_usage":{
                    "input_tokens":100,"output_tokens":20,"cached_input_tokens":10,
                    "reasoning_output_tokens":5
                }}}
            }),
        ];
        let dir = std::env::temp_dir().join("costdog_test_codex_classification");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout-test.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        for record in records {
            writeln!(file, "{}", record).unwrap();
        }

        let session = parse_codex_rollout(&path).unwrap();
        assert_eq!(*session.tool_calls.get("Bash").unwrap_or(&0), 1);
        assert_eq!(*session.tool_calls.get("Edit").unwrap_or(&0), 1);
        assert_eq!(*session.tool_calls.get("WebSearch").unwrap_or(&0), 1);
        assert_eq!(session.user_intent, "修复分类不准确的问题");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn classify_tool_ratios() {
        assert_eq!(classify(&tc(&[("Task", 5), ("Read", 5)]), None, "claude-code"), "agent");
        assert_eq!(classify(&tc(&[("WebSearch", 5), ("Read", 5)]), None, "claude-code"), "research");
        assert_eq!(classify(&tc(&[("Bash", 6), ("Read", 4)]), None, "claude-code"), "other");
        assert_eq!(classify(&tc(&[("Read", 7), ("Grep", 1), ("Write", 1)]), None, "claude-code"), "explore");
        // 写入工具本身无法区分修 Bug、做功能或改文档。
        assert_eq!(classify(&tc(&[("Edit", 6), ("Write", 4)]), None, "claude-code"), "other");
        assert_eq!(classify(&tc(&[("Write", 4), ("Read", 6)]), None, "claude-code"), "other");
    }

    #[test]
    fn test_ensure_db_migration_adds_activity_columns() {
        let dir = std::env::temp_dir().join(format!(
            "costdog_test_migration_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let db_path = dir.join("costdog.sqlite");
        let conn = ensure_db_exists_at(&db_path).expect("ensure_db_exists_at should succeed");
        for col in [
            "activity_category",
            "activity_category_override",
            "tool_calls",
            "git_branch",
            "project_key",
            "project_display",
        ] {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = ?1",
                rusqlite::params![col],
                |row| row.get(0),
            ).unwrap();
            assert_eq!(count, 1, "column {} should exist after migration", col);
        }
        for table in ["session_costs", "scan_files", "source_scan_status", "app_settings"] {
            let table_exists: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                rusqlite::params![table],
                |row| row.get(0),
            ).unwrap();
            assert_eq!(table_exists, 1, "table {table} should exist after migration");
        }
        drop(conn);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn v024_database_migrates_without_losing_sessions_or_alerts() {
        let dir = std::env::temp_dir().join(format!(
            "costdog_test_v024_migration_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("costdog.sqlite");
        {
            let legacy = rusqlite::Connection::open(&db_path).unwrap();
            legacy
                .execute_batch(
                    "CREATE TABLE sessions (
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
                        scanned_at TEXT,
                        PRIMARY KEY (session_id, source)
                    );
                    INSERT INTO sessions (
                        session_id, source, model, project, start_time, end_time,
                        input_tokens, output_tokens, cache_read_tokens,
                        cache_creation_tokens, reasoning_output_tokens,
                        disk_write_bytes, cost, scanned_at
                    ) VALUES (
                        'legacy-session', 'codex', 'gpt-legacy', '/tmp/legacy',
                        '2026-07-25T12:00:00Z', '2026-07-25T12:05:00Z',
                        100, 20, 5, 0, 3, 0, 1.25, '2026-07-25 12:06:00'
                    );
                    CREATE TABLE alerts (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        level TEXT NOT NULL,
                        message TEXT NOT NULL,
                        alert_key TEXT,
                        timestamp TEXT,
                        dismissed INTEGER DEFAULT 0
                    );
                    INSERT INTO alerts(level,message,alert_key,timestamp)
                      VALUES ('warn','old amount','daily_cost','2026-07-25 12:00:00');
                    INSERT INTO alerts(level,message,alert_key,timestamp)
                      VALUES ('warn','new amount','daily_cost','2026-07-25 12:01:00');",
                )
                .unwrap();
        }

        let conn = ensure_db_exists_at(&db_path).unwrap();
        let (date, input_tokens, cost): (String, u64, f64) = conn
            .query_row(
                "SELECT date, input_tokens, cost FROM sessions
                 WHERE session_id='legacy-session' AND source='codex'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(date, "2026-07-25");
        assert_eq!(input_tokens, 100);
        assert!((cost - 1.25).abs() < f64::EPSILON);

        let (ledger_cost, basis, completeness): (f64, String, String) = conn
            .query_row(
                "SELECT cost, cost_basis, usage_completeness FROM session_costs
                 WHERE session_id='legacy-session' AND source='codex'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert!((ledger_cost - 1.25).abs() < f64::EPSILON);
        assert_eq!(basis, "estimated");
        assert_eq!(completeness, "partial");

        let alert_count: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM alerts
                 WHERE alert_key='daily_cost' AND alert_period='2026-07-25'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(alert_count, 1);
        drop(conn);

        let second = ensure_db_exists_at(&db_path).unwrap();
        let session_count: u64 = second
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE session_id='legacy-session'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(session_count, 1);
        drop(second);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn date_ranges_use_inclusive_calendar_days() {
        let (start, end) = date_range(7);
        let start = chrono::NaiveDate::parse_from_str(&start, "%Y-%m-%d").unwrap();
        let end = chrono::NaiveDate::parse_from_str(&end, "%Y-%m-%d").unwrap();
        assert_eq!((end - start).num_days(), 6);

        let (start, end) = date_range(30);
        let start = chrono::NaiveDate::parse_from_str(&start, "%Y-%m-%d").unwrap();
        let end = chrono::NaiveDate::parse_from_str(&end, "%Y-%m-%d").unwrap();
        assert_eq!((end - start).num_days(), 29);
    }

    fn alert_test_connection() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE alerts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                level TEXT NOT NULL,
                message TEXT NOT NULL,
                alert_key TEXT,
                alert_period TEXT,
                timestamp TEXT DEFAULT (datetime('now')),
                dismissed INTEGER DEFAULT 0
            );
            CREATE UNIQUE INDEX idx_alerts_key_period_unique
              ON alerts(alert_key, alert_period);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn alert_upsert_is_unique_and_preserves_dismissal_within_period() {
        let conn = alert_test_connection();
        upsert_alert(&conn, "synthetic", "2026-07", "warn", "first").unwrap();
        let alert_id: i64 = conn
            .query_row("SELECT id FROM alerts", [], |row| row.get(0))
            .unwrap();
        dismiss_alert_in_connection(&conn, alert_id).unwrap();
        upsert_alert(&conn, "synthetic", "2026-07", "danger", "updated").unwrap();

        let (count, dismissed, level, message): (u64, bool, String, String) = conn
            .query_row(
                "SELECT COUNT(*), dismissed, level, message FROM alerts
                 WHERE alert_key='synthetic' AND alert_period='2026-07'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert!(dismissed);
        assert_eq!(level, "danger");
        assert_eq!(message, "updated");
    }

    #[test]
    fn dismiss_alert_reports_unknown_id() {
        let conn = alert_test_connection();
        let error = dismiss_alert_in_connection(&conn, 404).unwrap_err();
        assert!(error.contains("404"));
    }

    fn override_test_connection() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                session_id TEXT NOT NULL, source TEXT NOT NULL, date TEXT NOT NULL,
                model TEXT, project TEXT, project_key TEXT, project_display TEXT,
                start_time TEXT, end_time TEXT,
                input_tokens INTEGER DEFAULT 0, output_tokens INTEGER DEFAULT 0,
                cache_read_tokens INTEGER DEFAULT 0, cache_creation_tokens INTEGER DEFAULT 0,
                reasoning_output_tokens INTEGER DEFAULT 0, disk_write_bytes INTEGER DEFAULT 0,
                cost REAL DEFAULT 0, scanned_at TEXT, activity_category TEXT,
                activity_category_override TEXT, tool_calls TEXT, git_branch TEXT,
                PRIMARY KEY (session_id, source, date)
            );"
        ).unwrap();
        cost_ledger::ensure_schema(&conn).unwrap();
        conn
    }

    fn override_test_session(category: &str) -> SessionData {
        SessionData {
            session_id: "session-1".to_string(), source: "codex".to_string(),
            date: "2026-07-16".to_string(), model: "gpt-test".to_string(),
            project: "costdog".to_string(), start_time: "2026-07-16T08:00:00".to_string(),
            end_time: "2026-07-16T08:05:00".to_string(), input_tokens: 100,
            output_tokens: 20, cache_read_tokens: 5, cache_creation_tokens: 0,
            reasoning_tokens: 0, disk_write_bytes: 0, cost: 1.5,
            provider_cost_amount: None, usage_complete: true,
            tool_calls: HashMap::new(), git_branch: None,
            activity_category: category.to_string(), user_intent: String::new(),
        }
    }

    fn upsert_override_test_session(
        conn: &rusqlite::Connection,
        category: &str,
    ) -> Result<(), String> {
        let session = override_test_session(category);
        upsert_session(conn, &session)?;
        let record = build_cost_record(
            &session.model,
            session.input_tokens,
            session.output_tokens,
            session.cache_read_tokens,
            session.cache_creation_tokens,
            session.reasoning_tokens,
            Some(session.cost),
            session.usage_complete,
            None,
        );
        cost_ledger::upsert(
            conn,
            &session.session_id,
            &session.source,
            &session.date,
            &record,
        )
    }

    #[test]
    fn activity_override_survives_session_rescan_and_can_be_cleared() {
        let conn = override_test_connection();
        upsert_override_test_session(&conn, "feature").unwrap();
        set_activity_category_override_in_connection(
            &conn, "session-1", "codex", "2026-07-16", Some("docs"),
        ).unwrap();

        upsert_override_test_session(&conn, "bugfix").unwrap();
        let (automatic, manual, effective): (String, Option<String>, String) = conn.query_row(
            "SELECT activity_category, activity_category_override,
                    COALESCE(activity_category_override, activity_category, 'other')
             FROM sessions WHERE session_id='session-1' AND source='codex' AND date='2026-07-16'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(automatic, "bugfix");
        assert_eq!(manual.as_deref(), Some("docs"));
        assert_eq!(effective, "docs");

        set_activity_category_override_in_connection(
            &conn, "session-1", "codex", "2026-07-16", None,
        ).unwrap();
        let manual: Option<String> = conn.query_row(
            "SELECT activity_category_override FROM sessions WHERE session_id='session-1'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(manual, None);
    }

    #[test]
    fn activity_override_validates_category_and_full_session_key() {
        let conn = override_test_connection();
        upsert_override_test_session(&conn, "feature").unwrap();

        let invalid = set_activity_category_override_in_connection(
            &conn, "session-1", "codex", "2026-07-16", Some("custom"),
        ).unwrap_err();
        assert!(invalid.contains("Invalid activity category"));

        let missing = set_activity_category_override_in_connection(
            &conn, "session-1", "codex", "2026-07-17", Some("docs"),
        ).unwrap_err();
        assert!(missing.contains("Session not found"));
    }

    #[test]
    fn category_breakdown_uses_manual_override() {
        let conn = override_test_connection();
        upsert_override_test_session(&conn, "feature").unwrap();
        set_activity_category_override_in_connection(
            &conn, "session-1", "codex", "2026-07-16", Some("docs"),
        ).unwrap();

        let categories = get_cost_by_category(&conn, "2026-07-16", "2026-07-16").unwrap();
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].key, "docs");
        assert_eq!(categories[0].sessions, 1);
        assert!((categories[0].cost - 1.5).abs() < f64::EPSILON);
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
    fn unchanged_claude_jsonl_is_not_parsed_twice() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        source_status::ensure_schema(&conn).unwrap();
        let root = std::env::temp_dir().join(format!(
            "costdog_incremental_claude_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let project = root.join("test-project");
        std::fs::create_dir_all(&project).unwrap();
        let path = project.join("session.jsonl");
        let assistant = serde_json::json!({
            "type": "assistant",
            "sessionId": "incremental-session",
            "timestamp": "2026-07-27T08:00:00Z",
            "cwd": "/tmp/incremental",
            "message": {
                "model": "claude-test",
                "usage": {"input_tokens": 10, "output_tokens": 5},
                "content": []
            }
        });
        std::fs::write(&path, format!("not-json\n{assistant}\n")).unwrap();

        let first = scan_claude_directory(&conn, &root).unwrap();
        assert_eq!(first.sessions.len(), 1);
        assert_eq!(first.malformed_lines, 1);
        assert_eq!(first.skipped_files, 0);
        source_status::save_fingerprints(&conn, &first.fingerprints).unwrap();

        let second = scan_claude_directory(&conn, &root).unwrap();
        assert!(second.sessions.is_empty());
        assert_eq!(second.malformed_lines, 0);
        assert_eq!(second.skipped_files, 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn scan_claude_captures_first_real_user_intent() {
        use std::io::Write;

        let records = [
            serde_json::json!({
                "type":"user","sessionId":"intent-s1","timestamp":"2026-07-06T10:00:00Z",
                "message":{"content":"<environment_context>injected"}
            }),
            serde_json::json!({
                "type":"user","sessionId":"intent-s1","timestamp":"2026-07-06T10:00:01Z",
                "message":{"content":[{"type":"text","text":"first real request"}]}
            }),
            serde_json::json!({
                "type":"user","sessionId":"intent-s1","timestamp":"2026-07-06T10:00:02Z",
                "message":{"content":"later request must not replace it"}
            }),
        ];
        let dir = std::env::temp_dir().join("costdog_test_claude_intent");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        for record in records {
            writeln!(file, "{}", record).unwrap();
        }

        let sessions = parse_claude_jsonl(&path, "test-project");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].user_intent, "first real request");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_data_serde_skips_user_intent() {
        let session = SessionData {
            session_id: "s1".to_string(),
            source: "codex".to_string(),
            date: "2026-07-15".to_string(),
            model: "test".to_string(),
            project: "costdog".to_string(),
            start_time: String::new(),
            end_time: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            disk_write_bytes: 0,
            cost: 0.0,
            provider_cost_amount: None,
            usage_complete: true,
            tool_calls: HashMap::new(),
            git_branch: None,
            activity_category: "other".to_string(),
            user_intent: "must stay in memory".to_string(),
        };

        let json = serde_json::to_string(&session).unwrap();
        assert!(!json.contains("user_intent"));
        assert!(!json.contains("must stay in memory"));
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
