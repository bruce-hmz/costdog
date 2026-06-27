import * as fs from 'fs';
import * as path from 'path';
import { getCodexSessionsDir } from '../utils/paths';
import { SessionSummary, TokenUsage } from '../types';

interface CodexEventMsg {
  timestamp: string;
  type: 'event_msg';
  payload: {
    type: string;
    [key: string]: any;
  };
}

interface CodexSessionMeta {
  timestamp: string;
  type: 'session_meta';
  payload: {
    id: string;
    timestamp: string;
    cwd: string;
    originator: string;
    cli_version: string;
    source: string;
    model_provider: string;
    base_instructions: string;
    [key: string]: any;
  };
}

/**
 * Parse a single Codex rollout JSONL file
 */
export function parseCodexRollout(filePath: string): SessionSummary | null {
  try {
    const content = fs.readFileSync(filePath, 'utf-8');
    const lines = content.split('\n').filter(Boolean);

    if (lines.length === 0) return null;

    const tokenUsage: TokenUsage = {
      inputTokens: 0,
      outputTokens: 0,
      cacheReadTokens: 0,
      cacheCreationTokens: 0,
      reasoningOutputTokens: 0,
    };

    const toolCalls: Record<string, number> = {};
    let diskWriteBytes = 0;
    let model = '';
    let sessionId = '';
    let project = '';
    let startTime = '';
    let endTime = '';
    let lastTokenCount: any = null;

    for (const line of lines) {
      try {
        const obj = JSON.parse(line);

        // Session metadata
        if (obj.type === 'session_meta' && obj.payload) {
          const p = obj.payload;
          sessionId = p.id || '';
          project = p.cwd || '';
          model = p.model_provider || '';
          startTime = p.timestamp || obj.timestamp;
        }

        // Timestamp tracking
        if (obj.timestamp) {
          endTime = obj.timestamp;
        }

        // Token count events
        if (obj.type === 'event_msg' && obj.payload?.type === 'token_count' && obj.payload.info) {
          const info = obj.payload.info;
          const total = info.total_token_usage;
          if (total) {
            // These are cumulative totals for the session
            lastTokenCount = {
              inputTokens: total.input_tokens || 0,
              outputTokens: total.output_tokens || 0,
              cacheReadTokens: total.cached_input_tokens || 0,
              reasoningOutputTokens: total.reasoning_output_tokens || 0,
            };
          }
        }

        // Track turn_context for model info
        if (obj.type === 'turn_context' && obj.payload?.model) {
          model = obj.payload.model;
        }

        // Track tool calls from response_item
        if (obj.type === 'response_item' && obj.payload) {
          const p = obj.payload;
          if (p.type === 'function_call' || p.type === 'tool_call') {
            const name = p.name || p.function?.name || 'unknown';
            toolCalls[name] = (toolCalls[name] || 0) + 1;
          }
          // Check content array for tool calls
          if (p.content && Array.isArray(p.content)) {
            for (const block of p.content) {
              if (block.type === 'tool_use' || block.type === 'function_call') {
                const name = block.name || 'unknown';
                toolCalls[name] = (toolCalls[name] || 0) + 1;
              }
            }
          }
        }
      } catch {
        // Skip malformed lines
      }
    }

    // Use the last token count (cumulative) as the session total
    if (lastTokenCount) {
      // Codex total.input_tokens includes the cached portion; subtract it so the cost
      // calc (which bills cache read separately at 0.1x) doesn't double-count it.
      tokenUsage.inputTokens = Math.max(0, lastTokenCount.inputTokens - lastTokenCount.cacheReadTokens);
      tokenUsage.outputTokens = lastTokenCount.outputTokens;
      tokenUsage.cacheReadTokens = lastTokenCount.cacheReadTokens;
      tokenUsage.reasoningOutputTokens = lastTokenCount.reasoningOutputTokens;
    }

    const d = startTime ? new Date(startTime) : null;
    const date = d && !isNaN(d.getTime())
      ? `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
      : '';

    return {
      sessionId,
      source: 'codex',
      date,
      model,
      project: path.basename(project),
      startTime,
      endTime,
      tokenUsage,
      toolCalls,
      diskWriteBytes,
      cost: 0,
    };
  } catch {
    return null;
  }
}

/**
 * Scan all Codex sessions
 */
export function scanCodexSessions(): SessionSummary[] {
  const sessionsDir = getCodexSessionsDir();
  const results: SessionSummary[] = [];

  if (!fs.existsSync(sessionsDir)) return results;

  function walkDir(dir: string) {
    try {
      const entries = fs.readdirSync(dir, { withFileTypes: true });
      for (const entry of entries) {
        const fullPath = path.join(dir, entry.name);
        if (entry.isDirectory()) {
          walkDir(fullPath);
        } else if (entry.name.startsWith('rollout-') && entry.name.endsWith('.jsonl')) {
          const session = parseCodexRollout(fullPath);
          if (session) results.push(session);
        }
      }
    } catch {
      // Permission errors etc.
    }
  }

  walkDir(sessionsDir);
  return results;
}
