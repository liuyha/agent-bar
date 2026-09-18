import { describe, expect, it, vi } from 'vitest';
import type { TokenPeriod, TokenStatistics } from '../types';
import { calendarDate, dateRangeError, dateRangeLabel, emptyDateRange, hasDateRange, localStatisticsRange } from './statisticsDateRange';

function day(date: string, totalTokens: number, overrides: Partial<TokenPeriod> = {}): TokenPeriod {
  return {
    period: 'day', startAt: new Date(`${date}T00:00:00`).toISOString(), endAt: new Date(`${date}T23:59:59.999`).toISOString(),
    inputTokens: totalTokens - 2, cachedInputTokens: 1, cacheWriteTokens: 1, outputTokens: 2,
    totalTokens, estimatedCostUsd: 0.1, unpricedTokens: 0, requestCount: 1, conversationTurns: 2, ...overrides,
  };
}

function statistics(dailyPeriods: TokenStatistics['dailyPeriods']): TokenStatistics {
  return {
    status: 'ready', message: null, updatedAt: '2026-09-18T00:00:00Z', dailyPeriods,
    periods: [{ ...day('2025-12-31', 999), period: 'all' }],
  };
}

describe('statistics date range validation', () => {
  const now = new Date(2026, 8, 18, 14);

  it.each([
    ['', '', null],
    ['2026-09-18', '2026-09-18', null],
    ['2024-02-29', '', null],
    ['', '2026-09-18', null],
    ['2026-09-19', '', '日期不能晚于今天。'],
    ['', '2026-09-19', '日期不能晚于今天。'],
    ['2026-09-18', '2026-09-17', '开始日期不能晚于结束日期。'],
    ['2026-02-29', '', '请输入有效日期。'],
    ['', '2026-04-31', '请输入有效日期。'],
    ['2026-9-01', '', '请输入有效日期。'],
    ['not-a-date', '', '请输入有效日期。'],
  ])('validates %s through %s', (startDate, endDate, error) => {
    expect(dateRangeError({ startDate, endDate }, now)).toBe(error);
  });

  it('labels open boundaries and detects only active filters', () => {
    expect(hasDateRange(emptyDateRange)).toBe(false);
    expect(hasDateRange({ startDate: '2026-09-01', endDate: '' })).toBe(true);
    expect(hasDateRange({ startDate: '', endDate: '2026-09-18' })).toBe(true);
    expect(dateRangeLabel({ startDate: '', endDate: '2026-09-18' })).toBe('最早记录 – 2026-09-18');
    expect(dateRangeLabel({ startDate: '2026-09-01', endDate: '' })).toBe('2026-09-01 – 今日');
  });
});

describe('local statistics date ranges', () => {
  const now = new Date(2026, 8, 18, 14);

  it('includes both boundary days across a year boundary and aggregates every metric', () => {
    const source = statistics([day('2025-12-30', 100), day('2025-12-31', 10), day('2026-01-01', 20), day('2026-01-02', 200)]);
    expect(localStatisticsRange(source, { startDate: '2025-12-31', endDate: '2026-01-01' }, now)).toEqual({
      period: 'all', startAt: new Date(2025, 11, 31).toISOString(), endAt: new Date(2026, 0, 1, 23, 59, 59, 999).toISOString(),
      inputTokens: 26, cachedInputTokens: 2, cacheWriteTokens: 2, outputTokens: 4,
      totalTokens: 30, estimatedCostUsd: 0.2, unpricedTokens: 0, requestCount: 2, conversationTurns: 4,
    });
  });

  it.each([
    [{ startDate: '2026-09-18', endDate: '' }, 20],
    [{ startDate: '', endDate: '2026-09-17' }, 10],
    [emptyDateRange, 30],
  ])('accepts open boundaries and always excludes future buckets (%j)', (range, totalTokens) => {
    const result = localStatisticsRange(statistics([day('2026-09-17', 10), day('2026-09-18', 20), day('2026-09-19', 100)]), range, now);
    expect(result?.totalTokens).toBe(totalTokens);
    if (!range.endDate) expect(result?.endAt).toBe(now.toISOString());
    if (!range.startDate) expect(result?.startAt).toBe(new Date(2025, 11, 31).toISOString());
  });

  it('preserves unknown requests and turns while retaining the priced portion', () => {
    const source = statistics([
      day('2026-09-17', 10, { requestCount: null, estimatedCostUsd: null, unpricedTokens: 10 }),
      day('2026-09-18', 20, { conversationTurns: null, estimatedCostUsd: 0.25 }),
    ]);
    expect(localStatisticsRange(source, { startDate: '2026-09-17', endDate: '' }, now)).toMatchObject({
      totalTokens: 30, estimatedCostUsd: 0.25, unpricedTokens: 10, requestCount: null, conversationTurns: null,
    });
    expect(localStatisticsRange(source, { startDate: '2026-09-17', endDate: '2026-09-17' }, now)).toMatchObject({
      estimatedCostUsd: null, unpricedTokens: 10, requestCount: null, conversationTurns: 2,
    });
  });

  it('returns measured zeros when a valid local range has no matching buckets', () => {
    expect(localStatisticsRange(statistics([day('2026-09-17', 10)]), { startDate: '2026-09-18', endDate: '' }, now)).toMatchObject({
      totalTokens: 0, estimatedCostUsd: 0, unpricedTokens: 0, requestCount: 0, conversationTurns: 0,
    });
  });

  it.each([undefined, null])('does not infer zeros from missing daily history (%s)', (dailyPeriods) => {
    expect(localStatisticsRange(statistics(dailyPeriods), { startDate: '2026-09-18', endDate: '' }, now)).toBeNull();
  });

  it('rejects invalid ranges and unavailable statistics', () => {
    const source = statistics([]);
    expect(localStatisticsRange(source, { startDate: '2026-09-18', endDate: '2026-09-17' }, now)).toBeNull();
    expect(localStatisticsRange(source, { startDate: '', endDate: '2026-09-19' }, now)).toBeNull();
    expect(localStatisticsRange({ ...source, status: 'error' }, emptyDateRange, now)).toBeNull();
    expect(localStatisticsRange({ ...source, status: 'unavailable' }, emptyDateRange, now)).toBeNull();
  });

  it.each([
    ['Asia/Shanghai', '2026-09-16T16:01:00Z', '2026-09-17'],
    ['America/Los_Angeles', '2026-09-17T06:59:00Z', '2026-09-16'],
  ])('compares local dates rather than UTC prefixes in %s', (timezone, instant, selectedDate) => {
    vi.stubEnv('TZ', timezone);
    try {
      const localNow = new Date('2026-09-18T12:00:00Z');
      const source = statistics([day('2026-09-15', 10, { startAt: instant })]);
      expect(calendarDate(new Date(instant))).toBe(selectedDate);
      expect(localStatisticsRange(source, { startDate: selectedDate, endDate: selectedDate }, localNow)?.totalTokens).toBe(10);
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it('does not mutate the source, date range, or supplied current date', () => {
    const source = statistics([day('2026-09-18', 20), day('2026-09-17', 10)]);
    const before = structuredClone(source);
    source.dailyPeriods!.forEach(Object.freeze);
    Object.freeze(source.dailyPeriods);
    Object.freeze(source.periods);
    Object.freeze(source);
    const range = Object.freeze({ startDate: '2026-09-17', endDate: '2026-09-18' });
    const timestamp = now.getTime();
    expect(localStatisticsRange(source, range, now)?.totalTokens).toBe(30);
    expect(source).toEqual(before);
    expect(now.getTime()).toBe(timestamp);
  });
});
