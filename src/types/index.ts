export type ProviderId = 'codex' | 'claude';
export type Theme = 'system' | 'light' | 'dark';
export type CodexStatisticsSource = 'local' | 'auto' | 'oauth' | 'pat' | 'cli';
export type CodexStatisticsPreference = 'local' | 'auto';

export interface AppSettings {
  refreshIntervalSeconds: number;
  enabledProviders: ProviderId[];
  theme: Theme;
  codexStatisticsSource: CodexStatisticsPreference;
}

export interface UsageWindow {
  label: string;
  usedPercent: number;
  resetsAt: string | null;
}

export interface ResetCredit {
  id: string;
  remaining: number;
  expiresAt: string | null;
}

export interface ResetCredits {
  remaining: number | null;
  credits: ResetCredit[] | null;
  updatedAt: string | null;
  message: string | null;
}

export interface ProviderUsage {
  id: ProviderId;
  name: string;
  plan: string;
  source: 'local';
  status: 'ready' | 'unavailable' | 'error';
  account: string | null;
  message: string | null;
  windows: UsageWindow[];
  updatedAt: string | null;
  resetCredits?: ResetCredits | null;
}

export interface DashboardSnapshot {
  revision: number;
  providers: ProviderUsage[];
  updatedAt: string;
  mode: 'live';
}

export interface TokenPeriod {
  period: 'day' | 'week' | 'month' | 'year' | 'all';
  startAt: string;
  endAt: string;
  inputTokens: number;
  cachedInputTokens: number;
  cacheWriteTokens: number;
  outputTokens: number;
  totalTokens: number;
  estimatedCostUsd: number | null;
  unpricedTokens: number;
  requestCount: number | null;
  conversationTurns: number | null;
}

export interface TokenStatistics {
  status: 'ready' | 'unavailable' | 'error';
  message: string | null;
  periods: TokenPeriod[];
  updatedAt: string;
}

export interface AccountUsageSnapshot {
  source: CodexStatisticsSource;
  status: 'ready' | 'unavailable' | 'error';
  message: string | null;
  account: string | null;
  accountId: string | null;
  summary: {
    lifetimeTokens: number | null;
    peakDailyTokens: number | null;
    longestRunningTurnSec: number | null;
    currentStreakDays: number | null;
    longestStreakDays: number | null;
  };
  dailyUsage: { date: string; tokens: number }[] | null;
  serviceUpdatedAt: string | null;
  updatedAt: string | null;
}
