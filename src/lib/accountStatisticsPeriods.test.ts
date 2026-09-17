import { describe, expect, it, vi } from 'vitest';
import type { AccountUsageSnapshot, TokenPeriod } from '../types';
import { accountStatisticsPeriod } from './accountStatisticsPeriods';

function snapshot(dailyUsage: AccountUsageSnapshot['dailyUsage'], summary: Partial<AccountUsageSnapshot['summary']> = {}): AccountUsageSnapshot {
  return {
    source: 'oauth', status: 'ready', message: null, account: null, accountId: null,
    summary: { lifetimeTokens: null, peakDailyTokens: null, longestRunningTurnSec: null, currentStreakDays: null, longestStreakDays: null, ...summary },
    dailyUsage, serviceUpdatedAt: null, updatedAt: null,
  };
}

const periods: TokenPeriod['period'][] = ['day', 'week', 'month', 'year', 'all'];

describe('server statistics calendar periods', () => {
  const records = [
    { date: '2025-12-31', tokens: 1 },
    { date: '2026-01-01', tokens: 2 },
    { date: '2026-08-31', tokens: 4 },
    { date: '2026-09-01', tokens: 8 },
    { date: '2026-09-13', tokens: 16 },
    { date: '2026-09-14', tokens: 32 },
    { date: '2026-09-16', tokens: 64 },
    { date: '2026-09-17', tokens: 128 },
    { date: '2026-09-18', tokens: 256 },
  ];

  it.each([
    ['day', '2026-09-17', 128, 1],
    ['week', '2026-09-14', 224, 3],
    ['month', '2026-09-01', 248, 5],
    ['year', '2026-01-01', 254, 7],
    ['all', null, null, 8],
  ] as const)('filters %s through today and excludes future records', (period, startDate, totalTokens, count) => {
    const result = accountStatisticsPeriod(snapshot(records), period, new Date(2026, 8, 17, 15));
    expect(result).toMatchObject({ startDate, endDate: '2026-09-17', totalTokens, totalSource: period === 'all' ? null : 'daily', peakDailyTokens: period === 'all' ? null : 128, peakSource: period === 'all' ? null : 'daily' });
    expect(result.dailyUsage).toHaveLength(count);
  });

  it.each([
    [new Date(2026, 8, 14), '2026-09-14', '2026-09-14'],
    [new Date(2026, 8, 20), '2026-09-14', '2026-09-20'],
    [new Date(2026, 8, 1), '2026-08-31', '2026-09-01'],
    [new Date(2026, 0, 1), '2025-12-29', '2026-01-01'],
  ] as const)('starts the week on Monday, including month and year boundaries (%s)', (now, startDate, endDate) => {
    expect(accountStatisticsPeriod(snapshot(null), 'week', now)).toMatchObject({ startDate, endDate });
  });

  it('includes leap day and excludes the previous month', () => {
    const result = accountStatisticsPeriod(snapshot([
      { date: '2024-01-31', tokens: 100 },
      { date: '2024-02-01', tokens: 10 },
      { date: '2024-02-29', tokens: 20 },
      { date: '2024-03-01', tokens: 200 },
    ]), 'month', new Date(2024, 1, 29, 23, 59));
    expect(result).toMatchObject({ startDate: '2024-02-01', endDate: '2024-02-29', totalTokens: 30, peakDailyTokens: 20 });
    expect(result.dailyUsage?.map(({ date }) => date)).toEqual(['2024-02-01', '2024-02-29']);
  });

  it.each(periods)('preserves unknown, empty, and measured zero for %s', (period) => {
    const now = new Date(2026, 8, 17);
    expect(accountStatisticsPeriod(snapshot(null), period, now)).toMatchObject({ dailyUsage: null, totalTokens: null, totalSource: null, peakDailyTokens: null, peakSource: null });
    expect(accountStatisticsPeriod(snapshot([]), period, now)).toMatchObject({ dailyUsage: [], totalTokens: null, totalSource: null, peakDailyTokens: null, peakSource: null });
    const zero = snapshot([{ date: '2026-09-17', tokens: 0 }], { lifetimeTokens: 0, peakDailyTokens: 0 });
    expect(accountStatisticsPeriod(zero, period, now)).toMatchObject({ totalTokens: 0, totalSource: period === 'all' ? 'summary' : 'daily', peakDailyTokens: 0, peakSource: period === 'all' ? 'summary' : 'daily' });
  });

  it.each(periods.filter((period) => period !== 'all'))('does not substitute lifetime summaries for %s with no matching records', (period) => {
    const summary = { lifetimeTokens: 999, peakDailyTokens: 888 };
    const now = new Date(2026, 8, 17);
    expect(accountStatisticsPeriod(snapshot(null, summary), period, now)).toMatchObject({ dailyUsage: null, totalTokens: null, totalSource: null, peakDailyTokens: null, peakSource: null });
    expect(accountStatisticsPeriod(snapshot([{ date: '2025-12-31', tokens: 1 }], summary), period, now)).toMatchObject({ dailyUsage: [], totalTokens: null, totalSource: null, peakDailyTokens: null, peakSource: null });
    expect(accountStatisticsPeriod(snapshot([{ date: '2026-09-17', tokens: 12 }], summary), period, now)).toMatchObject({ totalTokens: 12, totalSource: 'daily', peakDailyTokens: 12, peakSource: 'daily' });
  });

  it.each([
    [{ lifetimeTokens: 1000, peakDailyTokens: 500 }, 1000, 'summary', 500, 'summary'],
    [{ lifetimeTokens: 0, peakDailyTokens: 0 }, 0, 'summary', 0, 'summary'],
    [{ lifetimeTokens: 1000, peakDailyTokens: null }, 1000, 'summary', null, null],
    [{ lifetimeTokens: null, peakDailyTokens: 500 }, null, null, 500, 'summary'],
    [{ lifetimeTokens: null, peakDailyTokens: null }, null, null, null, null],
  ] as const)('uses only all-time summaries and preserves missing values (%j)', (summary, totalTokens, totalSource, peakDailyTokens, peakSource) => {
    const result = accountStatisticsPeriod(snapshot([
      { date: '2020-01-01', tokens: 10 },
      { date: '2026-09-17', tokens: 20 },
      { date: '2026-09-18', tokens: 10000 },
    ], summary), 'all', new Date(2026, 8, 17));
    expect(result).toMatchObject({ startDate: null, totalTokens, totalSource, peakDailyTokens, peakSource });
    expect(result.dailyUsage).toHaveLength(2);
  });

  it('retains all-time summary values when no daily records were provided', () => {
    expect(accountStatisticsPeriod(snapshot(null, { lifetimeTokens: 500, peakDailyTokens: 100 }), 'all', new Date(2026, 8, 17)))
      .toMatchObject({ dailyUsage: null, totalTokens: 500, totalSource: 'summary', peakDailyTokens: 100, peakSource: 'summary' });
  });

  it.each([
    ['Asia/Shanghai', '2026-09-16T16:01:00Z', '2026-09-17'],
    ['America/Los_Angeles', '2026-09-17T06:59:00Z', '2026-09-16'],
  ])('uses the local calendar in %s around UTC midnight', (timezone, instant, expectedDate) => {
    vi.stubEnv('TZ', timezone);
    try {
      const now = new Date(instant);
      expect(now.getTimezoneOffset()).not.toBe(0);
      const result = accountStatisticsPeriod(snapshot([
        { date: '2026-09-16', tokens: 10 },
        { date: '2026-09-17', tokens: 20 },
      ]), 'day', now);
      expect(result).toMatchObject({ startDate: expectedDate, endDate: expectedDate });
      expect(result.dailyUsage).toEqual([{ date: expectedDate, tokens: expectedDate.endsWith('17') ? 20 : 10 }]);
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it('sorts the returned records without changing the snapshot, rows, or supplied date', () => {
    const dailyUsage = [{ date: '2026-09-17', tokens: 20 }, { date: '2026-09-14', tokens: 10 }];
    const source = snapshot(dailyUsage, { lifetimeTokens: 1000 });
    const before = structuredClone(source);
    for (const row of dailyUsage) Object.freeze(row);
    Object.freeze(dailyUsage);
    Object.freeze(source.summary);
    Object.freeze(source);
    const now = new Date(2026, 8, 17, 15, 32, 1);
    const timestamp = now.getTime();
    const result = accountStatisticsPeriod(source, 'week', now);
    expect(result.dailyUsage?.map(({ date }) => date)).toEqual(['2026-09-14', '2026-09-17']);
    expect(result.dailyUsage).not.toBe(dailyUsage);
    expect(source).toEqual(before);
    expect(now.getTime()).toBe(timestamp);
  });
});
