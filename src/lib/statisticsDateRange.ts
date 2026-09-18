import type { TokenPeriod, TokenStatistics } from '../types';

export interface StatisticsDateRange {
  startDate: string;
  endDate: string;
}

export const emptyDateRange: StatisticsDateRange = { startDate: '', endDate: '' };

export function calendarDate(date: Date): string {
  return `${date.getFullYear().toString().padStart(4, '0')}-${(date.getMonth() + 1).toString().padStart(2, '0')}-${date.getDate().toString().padStart(2, '0')}`;
}

export function hasDateRange(range: StatisticsDateRange): boolean {
  return Boolean(range.startDate || range.endDate);
}

export function dateRangeError(range: StatisticsDateRange, now = new Date()): string | null {
  for (const value of [range.startDate, range.endDate]) {
    if (!value) continue;
    const date = new Date(`${value}T00:00:00`);
    if (!/^\d{4}-\d{2}-\d{2}$/.test(value) || !Number.isFinite(date.getTime()) || calendarDate(date) !== value) return '请输入有效日期。';
    if (value > calendarDate(now)) return '日期不能晚于今天。';
  }
  if (range.startDate && range.endDate && range.startDate > range.endDate) return '开始日期不能晚于结束日期。';
  return null;
}

export function dateRangeLabel(range: StatisticsDateRange): string {
  return `${range.startDate || '最早记录'} – ${range.endDate || '今日'}`;
}

/** Sum already deduplicated native daily buckets, preserving unknown counts and partial prices. */
export function localStatisticsRange(statistics: TokenStatistics, range: StatisticsDateRange, now = new Date()): TokenPeriod | null {
  if (dateRangeError(range, now) || statistics.status !== 'ready' || !statistics.dailyPeriods) return null;
  const today = calendarDate(now);
  const endDate = range.endDate || today;
  const days = statistics.dailyPeriods.filter((day) => {
    const date = calendarDate(new Date(day.startAt));
    return date <= today && date <= endDate && (!range.startDate || date >= range.startDate);
  });
  const all = statistics.periods.find((period) => period.period === 'all');
  const end = new Date(`${endDate}T00:00:00`);
  end.setDate(end.getDate() + 1);
  end.setMilliseconds(-1);
  const result: TokenPeriod = {
    period: 'all',
    startAt: range.startDate ? new Date(`${range.startDate}T00:00:00`).toISOString() : all?.startAt ?? now.toISOString(),
    endAt: new Date(Math.min(end.getTime(), now.getTime())).toISOString(),
    inputTokens: 0, cachedInputTokens: 0, cacheWriteTokens: 0, outputTokens: 0,
    totalTokens: 0, estimatedCostUsd: 0, unpricedTokens: 0, requestCount: 0, conversationTurns: 0,
  };
  for (const day of days) {
    for (const field of ['inputTokens', 'cachedInputTokens', 'cacheWriteTokens', 'outputTokens', 'totalTokens', 'unpricedTokens'] as const) result[field] += day[field];
    result.estimatedCostUsd! += day.estimatedCostUsd ?? 0;
    for (const field of ['requestCount', 'conversationTurns'] as const) result[field] = result[field] === null || day[field] === null ? null : result[field] + day[field];
  }
  if (result.totalTokens > 0 && result.totalTokens === result.unpricedTokens) result.estimatedCostUsd = null;
  return result;
}
