import * as path from 'path';
import Database from 'better-sqlite3';
import { getOpencodeDbPath } from '../utils/paths';
import { SessionSummary, TokenUsage } from '../types';

/**
 * OpenCode (v1.14+) stores everything in a SQLite DB at
 * ~/.local/share/opencode/opencode.db. The `session` table carries pre-aggregated
 * cost + token columns written by the app itself, so we read it directly.
 *
 * Schema (sst/opencode dev):
 *   session.id, .directory, .title, .model (JSON {id,providerID,variant}),
 *   .cost, .tokens_input, .tokens_output, .tokens_reasoning,
 *   .tokens_cache_read, .tokens_cache_write, .time_created, .time_updated (ms epoch)
 *
 * Older versions may lack the token/cost columns — detected via PRAGMA
 * table_info and defaulted to 0. Opened READ-ONLY with busy_timeout so a running
 * OpenCode is never blocked.
 */

interface OpencodeSessionRow {
  id: string;
  directory: string | null;
  title: string | null;
  model: string | null;
  cost: number | null;
  tokens_input: number | null;
  tokens_output: number | null;
  tokens_reasoning: number | null;
  tokens_cache_read: number | null;
  tokens_cache_write: number | null;
  time_created: number | null;
  time_updated: number | null;
}

function localDate(ms: number): string {
  const d = new Date(ms);
  if (isNaN(d.getTime())) return '';
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}

function isoFromMs(ms: number | null, fallback: number | null): string {
  const v = ms ?? fallback;
  if (v == null) return '';
  const d = new Date(v);
  return isNaN(d.getTime()) ? '' : d.toISOString();
}

/** Extract a model id from the JSON `model` column; tolerate plain strings / null. */
function parseModelId(raw: string | null): string {
  if (!raw) return '';
  try {
    const o = JSON.parse(raw);
    if (typeof o === 'string') return o;
    return o?.id || o?.modelID || '';
  } catch {
    return raw;
  }
}

/** Open the OpenCode DB read-only. Returns null if missing / unreadable / no session table. */
function openOpencodeDb(): Database.Database | null {
  const dbPath = getOpencodeDbPath();
  const fs = require('fs');
  if (!fs.existsSync(dbPath)) return null;
  try {
    const db = new Database(dbPath, { readonly: true, timeout: 5000, fileMustExist: true });
    db.pragma('busy_timeout = 5000');
    const tables = db.prepare("SELECT name FROM sqlite_master WHERE type='table'").all() as { name: string }[];
    if (!tables.some(t => t.name === 'session')) return null;
    return db;
  } catch {
    return null;
  }
}

export function scanOpencodeSessions(): SessionSummary[] {
  const db = openOpencodeDb();
  if (!db) return [];

  try {
    // Detect which columns exist so older DBs (without tokens_*/cost) degrade gracefully.
    const cols = db.prepare('PRAGMA table_info(session)').all() as { name: string }[];
    const has = (n: string) => cols.some(c => c.name === n);
    const selectCost = has('cost') ? 'cost' : '0 AS cost';
    const selectTi = has('tokens_input') ? 'tokens_input' : '0 AS tokens_input';
    const selectTo = has('tokens_output') ? 'tokens_output' : '0 AS tokens_output';
    const selectTr = has('tokens_reasoning') ? 'tokens_reasoning' : '0 AS tokens_reasoning';
    const selectCrr = has('tokens_cache_read') ? 'tokens_cache_read' : '0 AS tokens_cache_read';
    const selectCw = has('tokens_cache_write') ? 'tokens_cache_write' : '0 AS tokens_cache_write';

    const rows = db.prepare(`
      SELECT id, directory, title, model, ${selectCost}, ${selectTi}, ${selectTo},
             ${selectTr}, ${selectCrr}, ${selectCw}, time_created, time_updated
      FROM session
    `).all() as OpencodeSessionRow[];

    const out: SessionSummary[] = [];
    for (const r of rows) {
      const startedMs = r.time_created ?? r.time_updated ?? 0;
      const usage: TokenUsage = {
        inputTokens: r.tokens_input || 0,
        outputTokens: r.tokens_output || 0,
        cacheReadTokens: r.tokens_cache_read || 0,
        // OpenCode calls it cache_write; cost calc bills it like Anthropic cache creation.
        cacheCreationTokens: r.tokens_cache_write || 0,
        reasoningOutputTokens: r.tokens_reasoning || 0,
      };
      const total =
        usage.inputTokens + usage.outputTokens + usage.cacheReadTokens +
        usage.cacheCreationTokens + usage.reasoningOutputTokens;
      if (total === 0 && !(r.cost && r.cost > 0)) continue; // skip noise (matches the scan-time filter)

      out.push({
        sessionId: r.id,
        source: 'opencode',
        date: localDate(startedMs),
        model: parseModelId(r.model) || 'unknown',
        project: path.basename(r.directory || '') || 'unknown',
        startTime: isoFromMs(r.time_created, r.time_updated),
        endTime: isoFromMs(r.time_updated, r.time_created),
        tokenUsage: usage,
        toolCalls: {},
        diskWriteBytes: 0,
        // Prefer the app's own cost (it knows its real provider pricing); aggregator
        // recomputes from tokens when this is 0.
        cost: r.cost || 0,
      });
    }
    return out;
  } catch {
    return [];
  } finally {
    try { db.close(); } catch { /* ignore */ }
  }
}
