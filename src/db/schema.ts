import Database from 'better-sqlite3';
import { getCostDogDbPath, getCostDogConfigDir } from '../utils/paths';
import * as fs from 'fs';

let _db: Database.Database | null = null;

export function getDb(): Database.Database {
  if (_db) return _db;

  const dbPath = getCostDogDbPath();
  fs.mkdirSync(getCostDogConfigDir(), { recursive: true });

  _db = new Database(dbPath);
  _db.pragma('journal_mode = WAL');
  _db.pragma('synchronous = NORMAL');

  // Create tables
  _db.exec(`
    CREATE TABLE IF NOT EXISTS sessions (
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

    CREATE TABLE IF NOT EXISTS daily_cache (
      date TEXT NOT NULL,
      sessions INTEGER DEFAULT 0,
      input_tokens INTEGER DEFAULT 0,
      output_tokens INTEGER DEFAULT 0,
      cache_read_tokens INTEGER DEFAULT 0,
      cost REAL DEFAULT 0,
      disk_write_bytes INTEGER DEFAULT 0,
      PRIMARY KEY (date)
    );

    CREATE TABLE IF NOT EXISTS alerts (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      level TEXT NOT NULL,
      message TEXT NOT NULL,
      alert_key TEXT,
      timestamp TEXT DEFAULT (datetime('now')),
      dismissed INTEGER DEFAULT 0
    );
  `);

  // Migration: CREATE TABLE IF NOT EXISTS won't add alert_key to an existing table.
  const alertCols = _db.prepare('PRAGMA table_info(alerts)').all() as { name: string }[];
  if (!alertCols.some(c => c.name === 'alert_key')) {
    _db.exec('ALTER TABLE alerts ADD COLUMN alert_key TEXT');
  }

  return _db;
}

/**
 * Upsert a session record
 */
export function upsertSession(s: {
  sessionId: string;
  source: string;
  model: string;
  project: string;
  startTime: string;
  endTime: string;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  reasoningOutputTokens: number;
  diskWriteBytes: number;
  cost: number;
}) {
  const db = getDb();
  db.prepare(`
    INSERT INTO sessions (session_id, source, model, project, start_time, end_time,
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
      scanned_at = datetime('now')
  `).run(
    s.sessionId, s.source, s.model, s.project, s.startTime, s.endTime,
    s.inputTokens, s.outputTokens, s.cacheReadTokens, s.cacheCreationTokens,
    s.reasoningOutputTokens, s.diskWriteBytes, s.cost,
  );
}

/**
 * Get daily summary for a date range
 */
export function getDailySummary(startDate: string, endDate: string) {
  const db = getDb();
  return db.prepare(`
    SELECT
      date(start_time) as date,
      COUNT(*) as sessions,
      SUM(input_tokens) as input_tokens,
      SUM(output_tokens) as output_tokens,
      SUM(cache_read_tokens) as cache_read_tokens,
      SUM(cost) as cost,
      SUM(disk_write_bytes) as disk_write_bytes
    FROM sessions
    WHERE date(start_time) >= ? AND date(start_time) <= ?
    GROUP BY date(start_time)
    ORDER BY date(start_time) DESC
  `).all(startDate, endDate);
}

/**
 * Get aggregate stats for a date range
 */
export function getAggregateStats(startDate: string, endDate: string) {
  const db = getDb();
  return db.prepare(`
    SELECT
      COUNT(*) as sessions,
      COALESCE(SUM(input_tokens), 0) as input_tokens,
      COALESCE(SUM(output_tokens), 0) as output_tokens,
      COALESCE(SUM(cache_read_tokens), 0) as cache_read_tokens,
      COALESCE(SUM(disk_write_bytes), 0) as disk_write_bytes,
      COALESCE(SUM(cost), 0) as cost
    FROM sessions
    WHERE date(start_time) >= ? AND date(start_time) <= ?
  `).get(startDate, endDate) as any;
}

/**
 * Get top models for a date range
 */
export function getTopModels(startDate: string, endDate: string, limit = 5) {
  const db = getDb();
  return db.prepare(`
    SELECT
      model,
      COUNT(*) as calls,
      SUM(input_tokens) as input_tokens,
      SUM(output_tokens) as output_tokens,
      SUM(cost) as cost
    FROM sessions
    WHERE date(start_time) >= ? AND date(start_time) <= ?
    GROUP BY model
    ORDER BY cost DESC
    LIMIT ?
  `).all(startDate, endDate, limit);
}

/**
 * Get recent sessions
 */
export function getRecentSessions(limit = 20) {
  const db = getDb();
  return db.prepare(`
    SELECT * FROM sessions
    ORDER BY start_time DESC
    LIMIT ?
  `).all(limit);
}

/**
 * Add an alert
 */
export function addAlert(key: string, level: string, message: string) {
  const db = getDb();
  // One row per (key, day): refresh today's existing row (keeps amount fresh), else insert.
  const res = db.prepare(
    `UPDATE alerts SET message = ?, level = ?, timestamp = datetime('now','localtime'), dismissed = 0
     WHERE alert_key = ? AND date(timestamp) = date('now','localtime')`
  ).run(message, level, key);
  if (res.changes === 0) {
    db.prepare(
      `INSERT INTO alerts (level, message, alert_key, timestamp) VALUES (?, ?, ?, datetime('now','localtime'))`
    ).run(level, message, key);
  }
}

/**
 * Get active alerts
 */
export function getAlerts(limit = 10) {
  const db = getDb();
  return db.prepare(`
    SELECT * FROM alerts WHERE dismissed = 0
    ORDER BY timestamp DESC LIMIT ?
  `).all(limit);
}
