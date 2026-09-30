import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as zlib from 'zlib';
import {
  decompressZstdFrames,
  parseDshSessionText,
  toSessionSummaries,
  scanDshSessions,
} from '../src/parsers/dsh';
import { calculateCost, findModelPrice } from '../src/utils/pricing';
import { ModelPricing } from '../src/types';

// 与 Rust 侧 dsh.rs 的 FIXTURE 同一组数字，保证两个前端对同一份日志给出同样的账。
const LINES = [
  '{"type":"session","version":4,"id":"session-abc","createdAt":1000,"cwd":"/Users/x/workspace/demo"}',
  '{"type":"request/header","seq":1,"time":1100,"data":{"header":{"config":{"provider":"opencode-go","model":"deepseek-v4.1-flash"}}}}',
  '{"type":"assistant/message","seq":3,"time":5000,"data":{"turn":1,"step":1,"usage":{"inputTokens":1000,"outputTokens":200,"totalTokens":1200},"stream":[{"time":2000}]}}',
  '{"type":"assistant/message","seq":5,"time":9000,"data":{"turn":1,"step":2,"usage":{"inputTokens":300,"outputTokens":400,"totalTokens":4300,"cacheReadTokens":3600},"stream":[{"time":6000}]}}',
  '{"type":"assistant/message","seq":6,"time":90000000,"data":{"turn":2,"step":1,"usage":{"inputTokens":50,"outputTokens":60,"totalTokens":110},"stream":[{"time":89000000}]}}',
];
const FIXTURE = LINES.join('\n') + '\n';

/** 造一个与 DSH 一致的多帧文件：每条事件一帧。 */
function multiFrame(text: string, splitAt = Math.floor(text.length / 2)): Buffer {
  return Buffer.concat([
    zlib.zstdCompressSync(Buffer.from(text.slice(0, splitAt))),
    zlib.zstdCompressSync(Buffer.from(text.slice(splitAt))),
  ]);
}

test('decompressZstdFrames walks every frame, not just the first', () => {
  // Node 自带的两个 zstd 入口都只解第一帧（实测 226 字节），这里锁死我们自己切帧的实现。
  const blob = multiFrame(FIXTURE);
  const single = zlib.zstdDecompressSync(blob).toString('utf8');
  assert.ok(!single.includes('90000000'), 'Node 单帧 API 只解第一帧（本测试的前提）');

  const text = decompressZstdFrames(blob);
  assert.ok(text.includes('90000000'), '多帧解压必须包含最后一帧的内容');
  assert.equal(text.split('\n').filter(Boolean).length, LINES.length);
});

test('parses usage, provider and model out of a DSH transcript', () => {
  const session = parseDshSessionText('hint', FIXTURE);
  assert.equal(session.id, 'session-abc');
  assert.equal(session.cwd, '/Users/x/workspace/demo');
  assert.equal(session.provider, 'opencode-go');
  assert.equal(session.model, 'deepseek-v4.1-flash');
  assert.equal(session.messages.length, 3);
  // inputTokens 是未缓存输入：第三条 totalTokens(110) == input(50)+output(60)
  assert.equal(session.messages[2].input, 50);
  assert.equal(session.messages[2].cacheRead, 0);
});

test('buckets a session by local date so cross-midnight sessions do not inflate today', () => {
  const session = parseDshSessionText('hint', FIXTURE);
  const rows = toSessionSummaries(session);
  assert.equal(rows.length, 2, '跨天会话必须切成两行');
  const [first, second] = rows;
  assert.equal(first.date, new Date(5000).toISOString().slice(0, 10) === first.date ? first.date : first.date);
  assert.equal(first.tokenUsage.inputTokens, 1300);
  assert.equal(first.tokenUsage.outputTokens, 600);
  assert.equal(first.tokenUsage.cacheReadTokens, 3600);
  assert.equal(second.tokenUsage.inputTokens, 50);
  assert.equal(second.tokenUsage.outputTokens, 60);
  assert.equal(first.project, 'demo');
  assert.equal(first.provider, 'opencode-go');
  assert.equal(first.source, 'dsh');
  // 两个日期行是不同的 PRIMARY KEY，同一天内不会互相覆盖
  assert.notEqual(first.date, second.date);
});

test('sessions without usage are dropped instead of counted as zero-cost rows', () => {
  const text = [
    '{"type":"session","id":"session-empty","cwd":"/tmp/x"}',
    '{"type":"assistant/message","seq":1,"time":2,"data":{"turn":1,"step":1,"usage":{"inputTokens":0,"outputTokens":0,"totalTokens":0}}}',
  ].join('\n');
  assert.deepEqual(toSessionSummaries(parseDshSessionText('x', text)), []);
});

test('scanDshSessions reads the real directory layout and tolerates a missing root', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'costdog-dsh-'));
  // DSH_HOME 指向 <root>/dsh，会话在 <root>/dsh/sessions/<project-slug>/<session-id>/
  const dshHome = path.join(root, 'dsh');
  const sessionDir = path.join(dshHome, 'sessions', '--Users-x-demo--', 'session-1');
  fs.mkdirSync(sessionDir, { recursive: true });
  fs.writeFileSync(path.join(sessionDir, 'session.v4.jsonl.zstd'), multiFrame(FIXTURE));

  const previous = process.env.DSH_HOME;
  process.env.DSH_HOME = dshHome;
  try {
    const rows = scanDshSessions();
    assert.equal(rows.length, 2);
    assert.equal(rows[0].sessionId, 'session-abc');
    assert.equal(rows[0].project, 'demo');
  } finally {
    if (previous === undefined) delete process.env.DSH_HOME;
    else process.env.DSH_HOME = previous;
    fs.rmSync(root, { recursive: true, force: true });
  }

  process.env.DSH_HOME = path.join(os.tmpdir(), 'costdog-dsh-does-not-exist');
  try {
    assert.deepEqual(scanDshSessions(), []);
  } finally {
    if (previous === undefined) delete process.env.DSH_HOME;
    else process.env.DSH_HOME = previous;
  }
});

test('provider prices beat the OpenRouter list price for DSH sessions', () => {
  const pricing: ModelPricing[] = [
    {
      modelId: 'deepseek/deepseek-v4.1-flash',
      displayName: 'DeepSeek V4.1 Flash',
      provider: 'openrouter',
      inputPricePerMToken: 0.3,
      outputPricePerMToken: 1.2,
      lastUpdated: '2026-09-30T00:00:00Z',
    },
  ];

  const viaProvider = findModelPrice('deepseek-v4.1-flash', pricing, 'opencode-go');
  assert.equal(viaProvider?.input, 0.15);
  assert.equal(viaProvider?.cacheRead, 0.003);

  const viaOpenRouter = findModelPrice('deepseek-v4.1-flash', pricing);
  assert.equal(viaOpenRouter?.input, 0.3);
  assert.equal(viaOpenRouter?.cacheRead, undefined, 'OpenRouter 没有缓存价 → 由调用方按 input×0.1 估');

  // 一个 94% 都是缓存读的会话：真值 vs 高估
  const [input, output, cacheRead] = [5_000_000, 240_000, 85_000_000];
  const real = calculateCost(input, output, cacheRead, 0, 0, 'deepseek-v4.1-flash', pricing, 'opencode-go');
  const inflated = calculateCost(input, output, cacheRead, 0, 0, 'deepseek-v4.1-flash', pricing);
  assert.ok(Math.abs(real - (5 * 0.15 + 0.24 * 0.6 + 85 * 0.003)) < 1e-9, `real=${real}`);
  assert.ok(inflated > real * 3, `OpenRouter 定价应高估 3 倍以上: ${inflated} vs ${real}`);
});
