use rusqlite::{params, Connection};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone)]
pub struct FileFingerprint {
    pub path: String,
    pub source: String,
    pub size_bytes: u64,
    pub modified_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct SourceScanMeasurement {
    pub source: String,
    pub detected: bool,
    pub records_found: u64,
    pub priced_records: u64,
    pub unpriced_records: u64,
    pub partial_records: u64,
    pub skipped_files: u64,
    pub malformed_lines: u64,
    pub duration_ms: u64,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SourceStatus {
    pub source: String,
    pub detected: bool,
    #[serde(rename = "displayPath")]
    pub display_path: String,
    #[serde(rename = "recordsFound")]
    pub records_found: u64,
    #[serde(rename = "pricedRecords")]
    pub priced_records: u64,
    #[serde(rename = "unpricedRecords")]
    pub unpriced_records: u64,
    #[serde(rename = "partialRecords")]
    pub partial_records: u64,
    #[serde(rename = "skippedFiles")]
    pub skipped_files: u64,
    #[serde(rename = "malformedLines")]
    pub malformed_lines: u64,
    #[serde(rename = "lastScanDurationMs")]
    pub last_scan_duration_ms: Option<u64>,
    #[serde(rename = "lastSuccessAt")]
    pub last_success_at: Option<String>,
    #[serde(rename = "lastError")]
    pub last_error: Option<String>,
}

pub fn ensure_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS scan_files (
            path TEXT PRIMARY KEY,
            source TEXT NOT NULL,
            size_bytes INTEGER NOT NULL,
            modified_at_ms INTEGER NOT NULL,
            last_scanned_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS source_scan_status (
            source TEXT PRIMARY KEY,
            detected INTEGER NOT NULL DEFAULT 0,
            records_found INTEGER NOT NULL DEFAULT 0,
            priced_records INTEGER NOT NULL DEFAULT 0,
            unpriced_records INTEGER NOT NULL DEFAULT 0,
            partial_records INTEGER NOT NULL DEFAULT 0,
            skipped_files INTEGER NOT NULL DEFAULT 0,
            malformed_lines INTEGER NOT NULL DEFAULT 0,
            last_scan_duration_ms INTEGER,
            last_success_at TEXT,
            last_error TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )
    .map_err(|error| error.to_string())?;
    for (column, ddl) in [
        (
            "skipped_files",
            "ALTER TABLE source_scan_status ADD COLUMN skipped_files INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "malformed_lines",
            "ALTER TABLE source_scan_status ADD COLUMN malformed_lines INTEGER NOT NULL DEFAULT 0",
        ),
    ] {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('source_scan_status') WHERE name=?1",
                params![column],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if exists == 0 {
            conn.execute(ddl, []).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

pub fn save_measurement(
    conn: &Connection,
    measurement: &SourceScanMeasurement,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO source_scan_status (
            source, detected, records_found, priced_records, unpriced_records,
            partial_records, skipped_files, malformed_lines, last_scan_duration_ms,
            last_success_at, last_error, updated_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
            CASE WHEN ?10 IS NULL THEN datetime('now','localtime') ELSE NULL END,
            ?10, datetime('now','localtime')
         )
         ON CONFLICT(source) DO UPDATE SET
            detected = excluded.detected,
            records_found = excluded.records_found,
            priced_records = excluded.priced_records,
            unpriced_records = excluded.unpriced_records,
            partial_records = excluded.partial_records,
            skipped_files = excluded.skipped_files,
            malformed_lines = excluded.malformed_lines,
            last_scan_duration_ms = excluded.last_scan_duration_ms,
            last_success_at = CASE
              WHEN excluded.last_error IS NULL THEN datetime('now','localtime')
              ELSE source_scan_status.last_success_at
            END,
            last_error = excluded.last_error,
            updated_at = datetime('now','localtime')",
        params![
            measurement.source,
            measurement.detected,
            measurement.records_found,
            measurement.priced_records,
            measurement.unpriced_records,
            measurement.partial_records,
            measurement.skipped_files,
            measurement.malformed_lines,
            measurement.duration_ms,
            measurement.error,
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn load_statuses(
    conn: &Connection,
    source_paths: &[(&str, PathBuf)],
) -> Result<Vec<SourceStatus>, String> {
    source_paths
        .iter()
        .map(|(source, path)| {
            conn.query_row(
                "SELECT detected, records_found, priced_records, unpriced_records,
                        partial_records, skipped_files, malformed_lines,
                        last_scan_duration_ms, last_success_at, last_error
                 FROM source_scan_status WHERE source = ?1",
                params![source],
                |row| {
                    Ok(SourceStatus {
                        source: (*source).to_string(),
                        detected: row.get(0)?,
                        display_path: display_path(path),
                        records_found: row.get(1)?,
                        priced_records: row.get(2)?,
                        unpriced_records: row.get(3)?,
                        partial_records: row.get(4)?,
                        skipped_files: row.get(5)?,
                        malformed_lines: row.get(6)?,
                        last_scan_duration_ms: row.get(7)?,
                        last_success_at: row.get(8)?,
                        last_error: row.get(9)?,
                    })
                },
            )
            .or_else(|error| {
                if error == rusqlite::Error::QueryReturnedNoRows {
                    Ok(SourceStatus {
                        source: (*source).to_string(),
                        detected: path.exists(),
                        display_path: display_path(path),
                        records_found: 0,
                        priced_records: 0,
                        unpriced_records: 0,
                        partial_records: 0,
                        skipped_files: 0,
                        malformed_lines: 0,
                        last_scan_duration_ms: None,
                        last_success_at: None,
                        last_error: None,
                    })
                } else {
                    Err(error)
                }
            })
            .map_err(|error| error.to_string())
        })
        .collect()
}

pub fn fingerprint(path: &Path, source: &str) -> Result<FileFingerprint, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("Cannot read JSONL metadata: {error}"))?;
    let modified_at_ms = metadata
        .modified()
        .map_err(|error| format!("Cannot read JSONL modification time: {error}"))?
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("JSONL modification time predates Unix epoch: {error}"))?
        .as_millis() as u64;
    Ok(FileFingerprint {
        path: path.to_string_lossy().to_string(),
        source: source.to_string(),
        size_bytes: metadata.len(),
        modified_at_ms,
    })
}

pub fn file_changed(conn: &Connection, fingerprint: &FileFingerprint) -> Result<bool, String> {
    let existing = conn.query_row(
        "SELECT size_bytes, modified_at_ms FROM scan_files WHERE path=?1",
        params![fingerprint.path],
        |row| Ok((row.get::<_, u64>(0)?, row.get::<_, u64>(1)?)),
    );
    match existing {
        Ok((size_bytes, modified_at_ms)) => Ok(
            size_bytes != fingerprint.size_bytes || modified_at_ms != fingerprint.modified_at_ms
        ),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(true),
        Err(error) => Err(error.to_string()),
    }
}

pub fn save_fingerprints(
    conn: &Connection,
    fingerprints: &[FileFingerprint],
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "INSERT INTO scan_files(path, source, size_bytes, modified_at_ms, last_scanned_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now','localtime'))
             ON CONFLICT(path) DO UPDATE SET
               source=excluded.source,
               size_bytes=excluded.size_bytes,
               modified_at_ms=excluded.modified_at_ms,
               last_scanned_at=excluded.last_scanned_at",
        )
        .map_err(|error| error.to_string())?;
    for fingerprint in fingerprints {
        stmt.execute(params![
            fingerprint.path,
            fingerprint.source,
            fingerprint.size_bytes,
            fingerprint.modified_at_ms,
        ])
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn refresh_measurement_counts(
    conn: &Connection,
    measurement: &mut SourceScanMeasurement,
) -> Result<(), String> {
    let (records, priced, unpriced, partial): (u64, u64, u64, u64) = conn
        .query_row(
            "SELECT
               COUNT(*),
               COALESCE(SUM(CASE WHEN sc.cost_basis IN ('provider','estimated') THEN 1 ELSE 0 END),0),
               COALESCE(SUM(CASE WHEN sc.cost_basis='unpriced' THEN 1 ELSE 0 END),0),
               COALESCE(SUM(CASE WHEN sc.usage_completeness='partial' THEN 1 ELSE 0 END),0)
             FROM sessions s
             JOIN session_costs sc
               ON sc.session_id=s.session_id AND sc.source=s.source AND sc.date=s.date
             WHERE s.source=?1",
            params![measurement.source],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|error| error.to_string())?;
    measurement.records_found = records;
    measurement.priced_records = priced;
    measurement.unpriced_records = unpriced;
    measurement.partial_records = partial;
    Ok(())
}

fn display_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(relative) = path.strip_prefix(home) {
            return format!("~/{}", relative.to_string_lossy());
        }
    }
    let components = path
        .components()
        .rev()
        .take(2)
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if components.is_empty() {
        return "Unknown path".to_string();
    }
    format!(
        "…/{}",
        components.into_iter().rev().collect::<Vec<_>>().join("/")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_measurement_clears_error_and_updates_success_time() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let mut measurement = SourceScanMeasurement {
            source: "codex".to_string(),
            detected: true,
            records_found: 0,
            priced_records: 0,
            unpriced_records: 0,
            partial_records: 0,
            skipped_files: 0,
            malformed_lines: 0,
            duration_ms: 4,
            error: Some("broken fixture".to_string()),
        };
        save_measurement(&conn, &measurement).unwrap();
        measurement.records_found = 3;
        measurement.priced_records = 2;
        measurement.error = None;
        save_measurement(&conn, &measurement).unwrap();

        let (error, success, records): (Option<String>, Option<String>, u64) = conn
            .query_row(
                "SELECT last_error, last_success_at, records_found
                 FROM source_scan_status WHERE source='codex'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(error, None);
        assert!(success.is_some());
        assert_eq!(records, 3);
    }

    #[test]
    fn unchanged_file_is_skipped_after_its_fingerprint_is_committed() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let path = std::env::temp_dir().join(format!(
            "costdog_fingerprint_{}_{}.jsonl",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::write(&path, b"first").unwrap();

        let first = fingerprint(&path, "codex").unwrap();
        assert!(file_changed(&conn, &first).unwrap());
        save_fingerprints(&conn, std::slice::from_ref(&first)).unwrap();
        assert!(!file_changed(&conn, &first).unwrap());

        fs::write(&path, b"second-version").unwrap();
        let changed = fingerprint(&path, "codex").unwrap();
        assert!(file_changed(&conn, &changed).unwrap());
        let _ = fs::remove_file(path);
    }
}
