import * as path from 'path';
import Database from 'better-sqlite3';
import { getZcodeDbPath } from '../utils/paths';
import { SessionSummary, TokenUsage } from '../types';

/**
 * ZCode CLI stores per-request model usage in a SQLite DB at ~/.zcode/cli/db/db.sqlite.
 *
 *   session     -> id, directory (project cwd), title, time_created, time_updated
 *   model_usage -> one row per model request: session_id, started_at (ms epoch),
 *                  model_id, status, input_tokens (NON-cached, Anthropic-style),
 *                  output_tokens, reasoning_tokens, cache_creation_input_tokens,
 *                  cache_read_input_tokens
 *
 * We aggregate model_usage by (session_id, LOCAL date of started_at) so a session
 * that spans midnight splits across days — same attribution rule as the Claude
 * Code parser. Opened READ-ONLY with a busy_timeout so a running ZCode app is
 * never blocked.
 */

interface ZcodeUsageRow {
  id: string;
  directory: string;
  started_at: number;
  model_id: string;
  input_tokens: number;
  output_tokens: number;
  reasoning_tokens: number;
  cache_creation_input_tokens: number;
  cache_read_input_tokens: number;
}

interface DayBucket {
  date: string;
  startTimeMs: number;
  endTimeMs: number;
  model: string;
  usage: TokenUsage;
}

function localDate(ms: number): string {
  const d = new Date(ms);
  if (isNaN(d.getTime())) return '';
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}

function isoFromMs(ms: number): string {
  return new Date(ms).toISOString();
}

/** Open the ZCode DB read-only. Returns null if missing / unreadable / wrong schema. */
function openZcodeDb(): Database.Database | null {
  const dbPath = getZcodeDbPath();
  // require('better-sqlite3') wants a file that exists for a read-only open
  const fs = require('fs');
  if (!fs.existsSync(dbPath)) return null;
  try {
    const db = new Database(dbPath, { readonly: true, timeout: 5000, fileMustExist: true });
    db.pragma('busy_timeout = 5000');
    // Verify the tables we depend on exist — a ZCode install that doesn't have
    // model_usage (older version) should degrade to "no data", not crash.
    const tables = db.prepare("SELECT name FROM sqlite_master WHERE type='table'").all() as { name: string }[];
    const names = new Set(tables.map(t => t.name));
    if (!names.has('model_usage') || !names.has('session')) return null;
    return db;
  } catch {
    return null;
  }
}

export function scanZcodeSessions(): SessionSummary[] {
  const db = openZcodeDb();
  if (!db) return [];

  try {
    const rows = db.prepare(`
      SELECT m.session_id AS id, s.directory AS directory, m.started_at AS started_at,
             m.model_id AS model_id, m.input_tokens AS input_tokens,
             m.output_tokens AS output_tokens, m.reasoning_tokens AS reasoning_tokens,
             m.cache_creation_input_tokens AS cache_creation_input_tokens,
             m.cache_read_input_tokens AS cache_read_input_tokens
      FROM model_usage m
      JOIN session s ON s.id = m.session_id
      WHERE m.status IN ('completed', 'error', 'cancelled')
        AND m.started_at IS NOT NULL
    `).all() as ZcodeUsageRow[];

    // Bucket by (session_id, local date of started_at)
    const buckets = new Map<string, DayBucket>();
    const projects = new Map<string, string>();

    for (const r of rows) {
      const date = localDate(r.started_at);
      if (!date) continue;
      const key = `${r.id}\u0000${date}`;
      let b = buckets.get(key);
      if (!b) {
        b = {
          date,
          startTimeMs: r.started_at,
          endTimeMs: r.started_at,
          model: r.model_id || '',
          usage: { inputTokens: 0, outputTokens: 0, cacheReadTokens: 0, cacheCreationTokens: 0, reasoningOutputTokens: 0 },
        };
        buckets.set(key, b);
      }
      if (r.started_at < b.startTimeMs) b.startTimeMs = r.started_at;
      if (r.started_at > b.endTimeMs) b.endTimeMs = r.started_at;
      if (r.model_id) b.model = r.model_id;
      b.usage.inputTokens += r.input_tokens || 0;
      b.usage.outputTokens += r.output_tokens || 0;
      b.usage.reasoningOutputTokens += r.reasoning_tokens || 0;
      b.usage.cacheCreationTokens += r.cache_creation_input_tokens || 0;
      b.usage.cacheReadTokens += r.cache_read_input_tokens || 0;
      if (!projects.has(r.id)) projects.set(r.id, r.directory || '');
    }

    const out: SessionSummary[] = [];
    for (const [compositeKey, b] of buckets) {
      // compositeKey is `${sessionId}\u0000${date}` — split off the sessionId to look up the project dir.
      const sessId = compositeKey.split('\u0000')[0];
      const dir = projects.get(sessId) || '';
      out.push({
        sessionId: sessId,
        source: 'zcode',
        date: b.date,
        model: b.model || 'unknown',
        project: path.basename(dir) || 'unknown',
        startTime: isoFromMs(b.startTimeMs),
        endTime: isoFromMs(b.endTimeMs),
        tokenUsage: b.usage,
        toolCalls: {},
        diskWriteBytes: 0,
        cost: 0, // filled in by aggregator via pricing
      });
    }
    return out;
  } catch {
    return [];
  } finally {
    try { db.close(); } catch { /* ignore */ }
  }
}
