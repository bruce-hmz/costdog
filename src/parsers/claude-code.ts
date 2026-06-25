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
      server_tool_use?: {
        web_search_requests?: number;
        web_fetch_requests?: number;
      };
      cache_creation?: {
        ephemeral_1h_input_tokens?: number;
        ephemeral_5m_input_tokens?: number;
      };
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

/**
 * Parse a single Claude Code session JSONL file
 */
export function parseSessionFile(filePath: string): SessionSummary | null {
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

    for (const line of lines) {
      try {
        const msg: ClaudeMessage = JSON.parse(line);

        // Track session metadata
        if (!sessionId && msg.sessionId) sessionId = msg.sessionId;
        if (!project && msg.cwd) project = msg.cwd;
        if (msg.timestamp) {
          if (!startTime) startTime = msg.timestamp;
          endTime = msg.timestamp;
        }

        // Extract token usage from assistant messages
        const usage = msg.message?.usage;
        if (usage) {
          tokenUsage.inputTokens += usage.input_tokens || 0;
          tokenUsage.outputTokens += usage.output_tokens || 0;
          tokenUsage.cacheReadTokens += usage.cache_read_input_tokens || 0;
          tokenUsage.cacheCreationTokens += usage.cache_creation_input_tokens || 0;
        }

        // Track model
        if (msg.message?.model) {
          model = msg.message.model;
        }

        // Track tool calls and disk writes
        const content = msg.message?.content || [];
        for (const block of content) {
          if (block.type === 'tool_use' && block.name) {
            toolCalls[block.name] = (toolCalls[block.name] || 0) + 1;

            // Calculate disk writes
            if (block.name === 'Write' && block.input?.content) {
              diskWriteBytes += Buffer.byteLength(block.input.content, 'utf-8');
            }
            if (block.name === 'Edit' && block.input?.new_string) {
              diskWriteBytes += Buffer.byteLength(block.input.new_string, 'utf-8');
            }
          }
        }
      } catch {
        // Skip malformed lines
      }
    }

    return {
      sessionId,
      source: 'claude-code',
      model,
      project: path.basename(project),
      startTime,
      endTime,
      tokenUsage,
      toolCalls,
      diskWriteBytes,
      cost: 0, // Will be calculated by pricing module
    };
  } catch {
    return null;
  }
}

/**
 * Scan all Claude Code sessions
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
          // Skip subagent directories
          if (entry.name === 'subagents') continue;
          walkDir(fullPath);
        } else if (entry.name.endsWith('.jsonl')) {
          const session = parseSessionFile(fullPath);
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
