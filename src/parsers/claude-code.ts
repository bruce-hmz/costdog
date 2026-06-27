import * as fs from 'fs';
import * as path from 'path';
import { getClaudeSessionsDir } from '../utils/paths';
import { SessionSummary, TokenUsage } from '../types';

interface ClaudeMessage {
  parentUuid: string | null;
  isSidechain: boolean;
  promptId: string;
  agentId?: string;
  type: 'user' | 'assistant' | 'attachment' | 'tool_result' | 'system';
  message: {
    role: 'user' | 'assistant';
    content: Array<{
      type: 'text' | 'thinking' | 'tool_use' | 'tool_result';
      text?: string;
      id?: string;
      name?: string;
      input?: Record<string, any>;
    }>;
    model?: string;
    usage?: {
      input_tokens: number;
      output_tokens: number;
      cache_creation_input_tokens?: number;
      cache_read_input_tokens?: number;
      server_tool_use?: { web_search_requests?: number; web_fetch_requests?: number };
      cache_creation?: { ephemeral_1h_input_tokens?: number; ephemeral_5m_input_tokens?: number };
      [key: string]: any;
    };
  };
  uuid: string;
  timestamp: string;
  userType: string;
  entrypoint: string;
  cwd: string;
  sessionId: string;
  version: string;
  gitBranch?: string;
}

/** Local YYYY-MM-DD of an ISO timestamp (so "today" matches the user's clock). */
function localDate(isoTs: string): string {
  if (!isoTs) return '';
  const d = new Date(isoTs);
  if (isNaN(d.getTime())) return '';
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}

interface DayBucket {
  date: string;
  startTime: string;
  endTime: string;
  model: string;
  usage: TokenUsage;
  diskWriteBytes: number;
}

/**
 * Parse a single Claude Code session JSONL file into per-day summaries.
 * Usage is bucketed by the LOCAL date of each assistant message, so a session that
 * spans midnight is split across days (and "today" reflects turns done today).
 */
export function parseSessionFile(filePath: string): SessionSummary[] {
  try {
    const content = fs.readFileSync(filePath, 'utf-8');
    const lines = content.split('\n').filter(Boolean);
    if (lines.length === 0) return [];

    const days = new Map<string, DayBucket>();
    const toolCalls: Record<string, number> = {};
    let sessionId = '';
    let project = '';

    const bucketFor = (ts: string, model: string): DayBucket | null => {
      const date = localDate(ts);
      if (!date) return null;
      let b = days.get(date);
      if (!b) {
        b = { date, startTime: ts, endTime: ts, model: model || '', usage: { inputTokens: 0, outputTokens: 0, cacheReadTokens: 0, cacheCreationTokens: 0, reasoningOutputTokens: 0 }, diskWriteBytes: 0 };
        days.set(date, b);
      }
      return b;
    };

    for (const line of lines) {
      let msg: ClaudeMessage;
      try { msg = JSON.parse(line); } catch { continue; }

      if (!sessionId && msg.sessionId) sessionId = msg.sessionId;
      if (!project && msg.cwd) project = msg.cwd;
      const ts = msg.timestamp || '';

      const model = msg.message?.model || '';
      const b = ts ? bucketFor(ts, model) : null;
      if (b) {
        if (ts < b.startTime) b.startTime = ts;
        if (ts > b.endTime) b.endTime = ts;
        if (model) b.model = model;
      }

      const usage = msg.message?.usage;
      if (usage && b) {
        b.usage.inputTokens += usage.input_tokens || 0;
        b.usage.outputTokens += usage.output_tokens || 0;
        b.usage.cacheReadTokens += usage.cache_read_input_tokens || 0;
        b.usage.cacheCreationTokens += usage.cache_creation_input_tokens || 0;
      }

      // Disk writes attribute to the day of the message
      const blocks = msg.message?.content || [];
      for (const block of blocks) {
        if (block.type === 'tool_use' && block.name) {
          toolCalls[block.name] = (toolCalls[block.name] || 0) + 1;
          if (block.name === 'Write' && block.input?.content) {
            if (b) b.diskWriteBytes += Buffer.byteLength(block.input.content, 'utf-8');
          }
          if (block.name === 'Edit' && block.input?.new_string) {
            if (b) b.diskWriteBytes += Buffer.byteLength(block.input.new_string, 'utf-8');
          }
        }
      }
    }

    const projectname = path.basename(project);
    const out: SessionSummary[] = [];
    for (const b of days.values()) {
      out.push({
        sessionId,
        source: 'claude-code',
        date: b.date,
        model: b.model || 'unknown',
        project: projectname,
        startTime: b.startTime,
        endTime: b.endTime,
        tokenUsage: b.usage,
        toolCalls,
        diskWriteBytes: b.diskWriteBytes,
        cost: 0,
      });
    }
    return out;
  } catch {
    return [];
  }
}

/**
 * Scan all Claude Code sessions. Returns per-day summaries (a long session yields
 * multiple entries, one per local day it was active).
 */
export function scanClaudeSessions(): SessionSummary[] {
  const sessionsDir = getClaudeSessionsDir();
  const results: SessionSummary[] = [];

  if (!fs.existsSync(sessionsDir)) return results;

  function walkDir(dir: string) {
    try {
      const entries = fs.readdirSync(dir, { withFileTypes: true });
      for (const entry of entries) {
        const fullPath = path.join(dir, entry.name);
        if (entry.isDirectory()) {
          if (entry.name === 'subagents') continue;
          walkDir(fullPath);
        } else if (entry.name.endsWith('.jsonl')) {
          results.push(...parseSessionFile(fullPath));
        }
      }
    } catch {
      // Permission errors etc.
    }
  }

  walkDir(sessionsDir);
  return results;
}
