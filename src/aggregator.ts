import { scanClaudeSessions } from './parsers/claude-code';
import { scanCodexSessions } from './parsers/codex';
import { loadPricing, calculateCost } from './utils/pricing';
import { upsertSession, getAggregateStats, getTopModels, getRecentSessions, getAlerts, addAlert } from './db/schema';
import { SessionSummary, DailySummary, DashboardData, Alert } from './types';

function dateRange(days: number): { start: string; end: string } {
  const end = new Date();
  const start = new Date();
  start.setDate(start.getDate() - days);
  return {
    start: start.toISOString().slice(0, 10),
    end: end.toISOString().slice(0, 10),
  };
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
      calls: m.calls || 0,
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
  const allSessions = [...claudeSessions, ...codexSessions];

  let newCount = 0;
  for (const s of allSessions) {
    const cost = calculateCost(
      s.tokenUsage.inputTokens,
      s.tokenUsage.outputTokens,
      s.tokenUsage.cacheReadTokens,
      s.model,
      pricing,
    );

    upsertSession({
      sessionId: s.sessionId,
      source: s.source,
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

  // Check for alerts
  checkAlerts(allSessions);

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

/**
 * Check for alert conditions
 */
function checkAlerts(sessions: SessionSummary[]) {
  const today = new Date().toISOString().slice(0, 10);
  const todaySessions = sessions.filter(s => s.startTime?.startsWith(today));

  // High daily cost alert
  const todayCost = todaySessions.reduce((sum, s) => sum + s.cost, 0);
  if (todayCost > 10) {
    addAlert('daily_cost', 'warn', `Daily cost exceeds $10: $${todayCost.toFixed(2)}`);
  }

  // High disk write alert (Codex logging bug detection)
  const todayDisk = todaySessions.reduce((sum, s) => sum + s.diskWriteBytes, 0);
  if (todayDisk > 100 * 1024 * 1024) { // 100 MB
    addAlert('disk_write', 'danger', `Excessive disk writes detected: ${(todayDisk / 1024 / 1024).toFixed(1)} MB today`);
  }

  // High token usage alert
  const todayTokens = todaySessions.reduce((sum, s) => sum + s.tokenUsage.inputTokens + s.tokenUsage.outputTokens, 0);
  if (todayTokens > 10_000_000) {
    addAlert('high_tokens', 'warn', `High token usage: ${(todayTokens / 1_000_000).toFixed(1)}M tokens today`);
  }
}
