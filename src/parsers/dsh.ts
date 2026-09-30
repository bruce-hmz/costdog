import * as fs from 'fs';
import * as path from 'path';
import * as zlib from 'zlib';
import { getDshSessionsDir } from '../utils/paths';
import { SessionSummary, TokenUsage } from '../types';

/**
 * DeepSeek Harness session transcripts.
 *
 * `~/.dsh/sessions/<project-slug>/<session-id>/session.v4.jsonl.zstd` — zstd-compressed
 * JSONL, and **every appended event is its own zstd frame** (a single live file measured
 * 1373 frames). Neither `zlib.zstdDecompressSync` nor the streaming API walks past the
 * first frame: both returned 226 bytes — exactly the `{"type":"session"}` record — which
 * silently makes every session look like it has no usage. So we split on the frame magic
 * ourselves and decode frame by frame.
 *
 * Usage semantics (verified against real transcripts):
 *   - `inputTokens` is the **uncached** input: `input + cacheRead == totalTokens - output`.
 *   - `totalTokens` is that step's total (it includes cacheRead), not a session running
 *     total — accumulate by summing steps.
 *   - Every step re-sends (and re-bills) the whole context, so summing steps is the bill.
 *   - Sessions span local days (a real one ran 22:14 → 09:12), so buckets are keyed by the
 *     local date of each message instead of dumping the whole session onto one day.
 */

const ZSTD_MAGIC = Buffer.from([0x28, 0xb5, 0x2f, 0xfd]);

/** zstd 帧头魔数在压缩数据里也可能偶然出现，所以解不开时把下一帧并进来重试。 */
const MAX_FRAME_MERGE = 4;

export function decompressZstdFrames(buf: Buffer): string {
  const starts: number[] = [];
  for (let i = buf.indexOf(ZSTD_MAGIC, 0); i >= 0; i = buf.indexOf(ZSTD_MAGIC, i + 4)) {
    starts.push(i);
  }
  if (starts.length === 0) return '';

  const parts: string[] = [];
  let index = 0;
  while (index < starts.length) {
    let next = index + 1;
    let decoded: string | null = null;
    while (decoded === null && next - index <= MAX_FRAME_MERGE) {
      const end = next < starts.length ? starts[next] : buf.length;
      try {
        decoded = zlib.zstdDecompressSync(buf.subarray(starts[index], end)).toString('utf8');
      } catch {
        // 可能是魔数误切（或文件尾巴写到一半），并上下一帧再试。
        if (next >= starts.length) break;
        next += 1;
      }
    }
    if (decoded !== null) {
      parts.push(decoded);
      index = next;
    } else {
      index += 1;
    }
  }
  return parts.join('');
}

interface DshUsage {
  inputTokens?: number;
  outputTokens?: number;
  cacheReadTokens?: number;
  cacheWriteTokens?: number;
}

interface DshMessage {
  timeMs: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
}

interface DshSessionText {
  id: string;
  cwd: string;
  provider: string;
  model: string;
  messages: DshMessage[];
}

function localDate(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** 解析（已解压的）会话文本。纯函数，便于单测。 */
export function parseDshSessionText(idHint: string, text: string): DshSessionText {
  const session: DshSessionText = {
    id: idHint,
    cwd: '',
    provider: '',
    model: '',
    messages: [],
  };

  for (const rawLine of text.split('\n')) {
    const line = rawLine.trim();
    if (!line) continue;
    let record: any;
    try {
      record = JSON.parse(line);
    } catch {
      continue;
    }

    if (record.type === 'session') {
      if (typeof record.id === 'string' && record.id) session.id = record.id;
      if (typeof record.cwd === 'string') session.cwd = record.cwd;
    } else if (record.type === 'request/header') {
      const config = record.data?.header?.config;
      if (config?.model) session.model = config.model;
      if (config?.provider) session.provider = config.provider;
    } else if (record.type === 'request/context') {
      if (!session.model && record.data?.model) session.model = record.data.model;
      if (!session.provider && record.data?.provider) session.provider = record.data.provider;
    } else if (record.type === 'assistant/message') {
      const usage: DshUsage | undefined = record.data?.usage;
      if (!usage) continue;
      const input = usage.inputTokens ?? 0;
      const output = usage.outputTokens ?? 0;
      const cacheRead = usage.cacheReadTokens ?? 0;
      const cacheWrite = usage.cacheWriteTokens ?? 0;
      if (input + output + cacheRead + cacheWrite === 0) continue;
      session.messages.push({
        timeMs: typeof record.time === 'number' ? record.time : 0,
        input,
        output,
        cacheRead,
        cacheWrite,
      });
    }
  }

  session.messages.sort((a, b) => a.timeMs - b.timeMs);
  return session;
}

/** 把一条会话按本地日期切成若干 SessionSummary（与 Rust 侧 scan_dsh_sessions 同口径）。 */
export function toSessionSummaries(session: DshSessionText): SessionSummary[] {
  const project = session.cwd ? path.basename(session.cwd) : '未命名';
  const model = session.model || 'unknown';
  const byDate = new Map<string, { usage: TokenUsage; start: number; end: number }>();

  for (const message of session.messages) {
    const date = localDate(message.timeMs);
    const bucket = byDate.get(date) ?? {
      usage: {
        inputTokens: 0,
        outputTokens: 0,
        cacheReadTokens: 0,
        cacheCreationTokens: 0,
        reasoningOutputTokens: 0,
      },
      start: message.timeMs,
      end: message.timeMs,
    };
    bucket.usage.inputTokens += message.input;
    bucket.usage.outputTokens += message.output;
    bucket.usage.cacheReadTokens += message.cacheRead;
    bucket.usage.cacheCreationTokens += message.cacheWrite;
    bucket.start = Math.min(bucket.start, message.timeMs);
    bucket.end = Math.max(bucket.end, message.timeMs);
    byDate.set(date, bucket);
  }

  const summaries: SessionSummary[] = [];
  for (const [date, bucket] of byDate) {
    summaries.push({
      sessionId: session.id,
      source: 'dsh',
      date,
      model,
      provider: session.provider || undefined,
      project,
      startTime: new Date(bucket.start).toISOString(),
      endTime: new Date(bucket.end).toISOString(),
      tokenUsage: bucket.usage,
      // DSH 的会话缓存/摘要里没有逐工具计数（只有完整 transcript 里有 tool/call 记录），
      // 与 Rust 侧一致先留空 → 活动分类落到 other。
      toolCalls: {},
      diskWriteBytes: 0,
      cost: 0,
    });
  }
  return summaries.sort((a, b) => a.date.localeCompare(b.date));
}

export function parseDshSessionFile(filePath: string, idHint: string): SessionSummary[] {
  try {
    const text = decompressZstdFrames(fs.readFileSync(filePath));
    if (!text) return [];
    return toSessionSummaries(parseDshSessionText(idHint, text));
  } catch {
    return [];
  }
}

/** 扫描 ~/.dsh/sessions/<project>/<session>/session.v4.jsonl.zstd */
export function scanDshSessions(): SessionSummary[] {
  const root = getDshSessionsDir();
  const results: SessionSummary[] = [];
  if (!fs.existsSync(root)) return results;

  let projects: fs.Dirent[];
  try {
    projects = fs.readdirSync(root, { withFileTypes: true });
  } catch {
    return results;
  }

  for (const project of projects) {
    if (!project.isDirectory()) continue;
    const projectDir = path.join(root, project.name);
    let sessions: fs.Dirent[];
    try {
      sessions = fs.readdirSync(projectDir, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of sessions) {
      if (!entry.isDirectory()) continue;
      const file = path.join(projectDir, entry.name, 'session.v4.jsonl.zstd');
      if (!fs.existsSync(file)) continue;
      results.push(...parseDshSessionFile(file, entry.name));
    }
  }
  return results;
}
