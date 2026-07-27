import { scanClaudeSessions } from './parsers/claude-code';
import { scanCodexSessions } from './parsers/codex';
import { scanZcodeSessions } from './parsers/zcode';
import { scanOpencodeSessions } from './parsers/opencode';
import { loadPricing, calculateCost } from './utils/pricing';
import { upsertSession, getAggregateStats, getTopModels, getRecentSessions, getAlerts } from './db/schema';
import { SessionSummary, DailySummary, DashboardData, Alert } from './types';

function localDay(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

export function dateRange(days: number, now = new Date()): { start: string; end: string } {
  const end = new Date(now);
  const start = new Date(now);
  const inclusiveDays = days <= 0 ? 1 : days;
  start.setDate(start.getDate() - (inclusiveDays - 1));
  return { start: localDay(start), end: localDay(end) };
}

function toDailySummary(stats: any, topModels: any[]): DailySummary {
  return {
    date: stats?.date || '',
    sessions: stats?.sessions || 0,
    tokenUsage: {
      inputTokens: stats?.input_tokens || 0,
      outputTokens: stats?.output_tokens || 0,
      cacheReadTokens: stats?.cache_read_tokens || 0,
      cacheCreationTokens: 0,
      reasoningOutputTokens: 0,
    },
    cost: stats?.cost || 0,
    diskWriteBytes: stats?.disk_write_bytes || 0,
    topModels: topModels.map((m: any) => ({
      model: m.model || 'unknown',
      sessions: m.sessions || 0,
      cost: m.cost || 0,
    })),
  };
}

/**
 * Full scan: parse all logs, calculate costs, store in DB
 */
export async function fullScan(): Promise<{ newSessions: number; totalSessions: number }> {
  const pricing = await loadPricing();

  const claudeSessions = scanClaudeSessions();
  const codexSessions = scanCodexSessions();
  const zcodeSessions = scanZcodeSessions();
  const opencodeSessions = scanOpencodeSessions();
  const allSessions = [...claudeSessions, ...codexSessions, ...zcodeSessions, ...opencodeSessions].filter(
    (s) =>
      s.tokenUsage.inputTokens +
        s.tokenUsage.outputTokens +
        s.tokenUsage.cacheReadTokens +
        s.tokenUsage.cacheCreationTokens +
        s.tokenUsage.reasoningOutputTokens >
      0,
  );

  let newCount = 0;
  for (const s of allSessions) {
    // OpenCode writes its own (provider-accurate) cost into the session row.
    // Trust it when present; otherwise recompute from tokens + our pricing table.
    const cost = s.cost > 0 ? s.cost : calculateCost(
      s.tokenUsage.inputTokens,
      s.tokenUsage.outputTokens,
      s.tokenUsage.cacheReadTokens,
      s.tokenUsage.cacheCreationTokens,
      s.tokenUsage.reasoningOutputTokens,
      s.model,
      pricing,
    );

    upsertSession({
      sessionId: s.sessionId,
      source: s.source,
      date: s.date,
      model: s.model,
      project: s.project,
      startTime: s.startTime,
      endTime: s.endTime,
      inputTokens: s.tokenUsage.inputTokens,
      outputTokens: s.tokenUsage.outputTokens,
      cacheReadTokens: s.tokenUsage.cacheReadTokens,
      cacheCreationTokens: s.tokenUsage.cacheCreationTokens,
      reasoningOutputTokens: s.tokenUsage.reasoningOutputTokens,
      diskWriteBytes: s.diskWriteBytes,
      cost,
    });
    newCount++;
  }

  return { newSessions: newCount, totalSessions: allSessions.length };
}

/**
 * Generate dashboard data
 */
export function getDashboardData(): DashboardData {
  const today = dateRange(0);
  const week = dateRange(7);
  const month = dateRange(30);
  const allTime = { start: '2000-01-01', end: '2099-12-31' };

  const todayStats = getAggregateStats(today.start, today.end);
  const weekStats = getAggregateStats(week.start, week.end);
  const monthStats = getAggregateStats(month.start, month.end);
  const allTimeStats = getAggregateStats(allTime.start, allTime.end);

  const todayModels = getTopModels(today.start, today.end);
  const weekModels = getTopModels(week.start, week.end);
  const monthModels = getTopModels(month.start, month.end);
  const allTimeModels = getTopModels(allTime.start, allTime.end);

  const recentSessions = getRecentSessions(20) as SessionSummary[];
  const alerts = getAlerts(10) as Alert[];

  return {
    today: toDailySummary(todayStats, todayModels),
    week: toDailySummary(weekStats, weekModels),
    month: toDailySummary(monthStats, monthModels),
    allTime: toDailySummary(allTimeStats, allTimeModels),
    recentSessions,
    alerts,
  };
}
