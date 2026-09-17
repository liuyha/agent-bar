export type ProviderId = 'codex' | 'claude';
export type Theme = 'system' | 'light' | 'dark';

export interface AppSettings {
  refreshIntervalSeconds: number;
  enabledProviders: ProviderId[];
  theme: Theme;
}

export interface UsageWindow {
  label: string;
  usedPercent: number;
  resetsAt: string | null;
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
