import type { AccountUsageSnapshot, TokenPeriod } from '../types';

export interface AccountStatisticsPeriod {
  startDate: string | null;
  endDate: string;
  dailyUsage: AccountUsageSnapshot['dailyUsage'];
  totalTokens: number | null;
  totalSource: 'summary' | 'daily' | null;
  peakDailyTokens: number | null;
  peakSource: 'summary' | 'daily' | null;
}

function calendarDate(date: Date): string {
  return `${date.getFullYear().toString().padStart(4, '0')}-${(date.getMonth() + 1).toString().padStart(2, '0')}-${date.getDate().toString().padStart(2, '0')}`;
}

export function accountStatisticsPeriod(
  snapshot: AccountUsageSnapshot,
  period: TokenPeriod['period'],
  now = new Date(),
): AccountStatisticsPeriod {
  const start = new Date(now);
  if (period === 'week') start.setDate(start.getDate() - (start.getDay() + 6) % 7);
  if (period === 'month' || period === 'year') start.setDate(1);
  if (period === 'year') start.setMonth(0);

  const startDate = period === 'all' ? null : calendarDate(start);
  const endDate = calendarDate(now);
  // Service dates are calendar dates: keep them as strings instead of parsing UTC midnight.
  const dailyUsage = snapshot.dailyUsage?.filter(({ date }) => date <= endDate && (startDate === null || date >= startDate))
    .sort((left, right) => left.date.localeCompare(right.date)) ?? null;
  const dailyTotal = dailyUsage?.length ? dailyUsage.reduce((total, day) => total + day.tokens, 0) : null;
  const dailyPeak = dailyUsage?.length ? dailyUsage.reduce((peak, day) => Math.max(peak, day.tokens), -Infinity) : null;
  // Returned daily records may cover only a portion of the account history.
  const totalTokens = period === 'all' ? snapshot.summary.lifetimeTokens : dailyTotal;
  const peakDailyTokens = period === 'all' ? snapshot.summary.peakDailyTokens : dailyPeak;

  return {
    startDate,
    endDate,
    dailyUsage,
    totalTokens,
    totalSource: totalTokens === null ? null : period === 'all' ? 'summary' : 'daily',
    peakDailyTokens,
    peakSource: peakDailyTokens === null ? null : period === 'all' ? 'summary' : 'daily',
  };
}
