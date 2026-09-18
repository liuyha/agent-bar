import type { AccountUsageSnapshot, TokenPeriod } from '../types';
import { calendarDate, dateRangeError, emptyDateRange, hasDateRange, type StatisticsDateRange } from './statisticsDateRange';

export interface AccountStatisticsPeriod {
  startDate: string | null;
  endDate: string;
  dailyUsage: AccountUsageSnapshot['dailyUsage'];
  totalTokens: number | null;
  totalSource: 'summary' | 'daily' | null;
  peakDailyTokens: number | null;
  peakSource: 'summary' | 'daily' | null;
}

export function accountStatisticsPeriod(
  snapshot: AccountUsageSnapshot,
  period: TokenPeriod['period'],
  now = new Date(),
  range: StatisticsDateRange = emptyDateRange,
): AccountStatisticsPeriod {
  const start = new Date(now);
  if (period === 'week') start.setDate(start.getDate() - (start.getDay() + 6) % 7);
  if (period === 'month' || period === 'year') start.setDate(1);
  if (period === 'year') start.setMonth(0);

  const filtered = period === 'all' && hasDateRange(range);
  const all = period === 'all' && !filtered;
  const valid = !filtered || !dateRangeError(range, now);
  const startDate = period === 'all' ? range.startDate || null : calendarDate(start);
  const endDate = filtered && range.endDate && range.endDate < calendarDate(now) ? range.endDate : calendarDate(now);
  // Service dates are calendar dates: keep them as strings instead of parsing UTC midnight.
  const dailyUsage = !valid ? null : snapshot.dailyUsage?.filter(({ date }) => date <= endDate && (startDate === null || date >= startDate))
    .sort((left, right) => left.date.localeCompare(right.date)) ?? null;
  const dailyTotal = dailyUsage?.length ? dailyUsage.reduce((total, day) => total + day.tokens, 0) : null;
  const dailyPeak = dailyUsage?.length ? dailyUsage.reduce((peak, day) => Math.max(peak, day.tokens), -Infinity) : null;
  // Returned daily records may cover only a portion of the account history.
  const totalTokens = all ? snapshot.summary.lifetimeTokens : dailyTotal;
  const peakDailyTokens = all ? snapshot.summary.peakDailyTokens : dailyPeak;

  return {
    startDate,
    endDate,
    dailyUsage,
    totalTokens,
    totalSource: totalTokens === null ? null : all ? 'summary' : 'daily',
    peakDailyTokens,
    peakSource: peakDailyTokens === null ? null : all ? 'summary' : 'daily',
  };
}
