/** Core types for CostDog */

export interface TokenUsage {
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  reasoningOutputTokens: number;
}

export interface ToolCall {
  name: string;
  count: number;
  diskWriteBytes: number;
}

export interface SessionSummary {
  sessionId: string;
  source: 'claude-code' | 'codex';
  model: string;
  project: string;
  startTime: string;
  endTime: string;
  tokenUsage: TokenUsage;
  toolCalls: Record<string, number>;
  diskWriteBytes: number;
  cost: number;
}

export interface DailySummary {
  date: string;
  sessions: number;
  tokenUsage: TokenUsage;
  cost: number;
  diskWriteBytes: number;
  topModels: { model: string; calls: number; cost: number }[];
}

export interface ModelPricing {
  modelId: string;
  displayName: string;
  provider: string;
  inputPricePerMToken: number;   // $ per million tokens
  outputPricePerMToken: number;
  cacheReadPricePerMToken?: number;
  lastUpdated: string;
}

export interface PricingDB {
  models: ModelPricing[];
  fetchedAt: string;
}

export interface DashboardData {
  today: DailySummary;
  week: DailySummary;
  month: DailySummary;
  allTime: DailySummary;
  recentSessions: SessionSummary[];
  alerts: Alert[];
}

export interface Alert {
  level: 'info' | 'warn' | 'danger';
  message: string;
  timestamp: string;
}
