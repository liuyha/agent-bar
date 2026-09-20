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
    ['day', '2026-09-16', '2026-09-16', 64, 64, 1],
    ['week', '2026-09-14', '2026-09-17', 224, 128, 3],
    ['month', '2026-09-01', '2026-09-17', 248, 128, 5],
    ['year', '2026-01-01', '2026-09-17', 254, 128, 7],
    ['all', null, '2026-09-17', null, null, 8],
  ] as const)('filters %s to its calendar range and excludes later records', (period, startDate, endDate, totalTokens, peakDailyTokens, count) => {
    const result = accountStatisticsPeriod(snapshot(records), period, new Date(2026, 8, 17, 15));
    expect(result).toMatchObject({ startDate, endDate, totalTokens, totalSource: period === 'all' ? null : 'daily', peakDailyTokens, peakSource: period === 'all' ? null : 'daily' });
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
    const zero = snapshot([{ date: '2026-09-16', tokens: 0 }], { lifetimeTokens: 0, peakDailyTokens: 0 });
    expect(accountStatisticsPeriod(zero, period, now)).toMatchObject({ totalTokens: 0, totalSource: period === 'all' ? 'summary' : 'daily', peakDailyTokens: 0, peakSource: period === 'all' ? 'summary' : 'daily' });
  });

  it.each(periods.filter((period) => period !== 'all'))('does not substitute lifetime summaries for %s with no matching records', (period) => {
    const summary = { lifetimeTokens: 999, peakDailyTokens: 888 };
    const now = new Date(2026, 8, 17);
    expect(accountStatisticsPeriod(snapshot(null, summary), period, now)).toMatchObject({ dailyUsage: null, totalTokens: null, totalSource: null, peakDailyTokens: null, peakSource: null });
    expect(accountStatisticsPeriod(snapshot([{ date: '2025-12-31', tokens: 1 }], summary), period, now)).toMatchObject({ dailyUsage: [], totalTokens: null, totalSource: null, peakDailyTokens: null, peakSource: null });
    expect(accountStatisticsPeriod(snapshot([{ date: '2026-09-16', tokens: 12 }], summary), period, now)).toMatchObject({ totalTokens: 12, totalSource: 'daily', peakDailyTokens: 12, peakSource: 'daily' });
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
    ['Asia/Shanghai', '2026-09-16T16:01:00Z', '2026-09-16'],
    ['America/Los_Angeles', '2026-09-17T06:59:00Z', '2026-09-15'],
  ])('uses the previous local calendar day in %s around UTC midnight', (timezone, instant, expectedDate) => {
    vi.stubEnv('TZ', timezone);
    try {
      const now = new Date(instant);
      expect(now.getTimezoneOffset()).not.toBe(0);
      const result = accountStatisticsPeriod(snapshot([
        { date: '2026-09-15', tokens: 10 },
        { date: '2026-09-16', tokens: 20 },
        { date: '2026-09-17', tokens: 40 },
      ]), 'day', now);
      expect(result).toMatchObject({ startDate: expectedDate, endDate: expectedDate });
      expect(result.dailyUsage).toEqual([{ date: expectedDate, tokens: expectedDate.endsWith('16') ? 20 : 10 }]);
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it.each([
    ['Asia/Shanghai', '2026-10-01T00:01:00', '2026-09-30'],
    ['Asia/Shanghai', '2026-01-01T00:01:00', '2025-12-31'],
    ['Asia/Shanghai', '2024-03-01T00:01:00', '2024-02-29'],
    ['America/Los_Angeles', '2026-03-09T00:30:00', '2026-03-08'],
    ['America/Los_Angeles', '2026-11-01T23:30:00', '2026-10-31'],
  ])('selects yesterday across calendar and DST boundaries in %s at %s', (timezone, instant, expectedDate) => {
    vi.stubEnv('TZ', timezone);
    try {
      const now = new Date(instant);
      const timestamp = now.getTime();
      const result = accountStatisticsPeriod(snapshot([{ date: expectedDate, tokens: 42 }]), 'day', now);
      expect(result).toMatchObject({ startDate: expectedDate, endDate: expectedDate, totalTokens: 42 });
      expect(result.dailyUsage).toEqual([{ date: expectedDate, tokens: 42 }]);
      expect(now.getTime()).toBe(timestamp);
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

describe('server statistics custom date ranges', () => {
  const now = new Date(2026, 8, 18, 14);
  const records = [
    { date: '2026-09-19', tokens: 160 },
    { date: '2026-09-18', tokens: 80 },
    { date: '2026-09-17', tokens: 40 },
    { date: '2026-01-01', tokens: 20 },
    { date: '2025-12-31', tokens: 10 },
    { date: '2025-12-30', tokens: 5 },
  ];
  const summary = { lifetimeTokens: 9999, peakDailyTokens: 8888 };

  it('includes both dates across a year boundary and derives metrics from matching records', () => {
    const result = accountStatisticsPeriod(snapshot(records, summary), 'all', now, { startDate: '2025-12-31', endDate: '2026-01-01' });
    expect(result).toEqual({
      startDate: '2025-12-31', endDate: '2026-01-01',
      dailyUsage: [{ date: '2025-12-31', tokens: 10 }, { date: '2026-01-01', tokens: 20 }],
      totalTokens: 30, totalSource: 'daily', peakDailyTokens: 20, peakSource: 'daily',
    });
  });

  it.each([
    [{ startDate: '2026-09-17', endDate: '' }, 120, 80, 2],
    [{ startDate: '', endDate: '2026-01-01' }, 35, 20, 3],
    [{ startDate: '2026-09-18', endDate: '2026-09-18' }, 80, 80, 1],
  ])('supports open and single-day ranges (%j)', (range, totalTokens, peakDailyTokens, count) => {
    const result = accountStatisticsPeriod(snapshot(records, summary), 'all', now, range);
    expect(result).toMatchObject({ totalTokens, peakDailyTokens, totalSource: 'daily', peakSource: 'daily' });
    expect(result.dailyUsage).toHaveLength(count);
    expect(result.endDate).toBe(range.endDate || '2026-09-18');
  });

  it.each([null, [], [{ date: '2026-09-17', tokens: 10 }]])('keeps missing range coverage unknown even with lifetime summaries (%j)', (dailyUsage) => {
    const result = accountStatisticsPeriod(snapshot(dailyUsage, summary), 'all', now, { startDate: '2026-09-18', endDate: '' });
    expect(result).toMatchObject({ totalTokens: null, totalSource: null, peakDailyTokens: null, peakSource: null });
    expect(result.dailyUsage).toEqual(dailyUsage === null ? null : []);
  });

  it('retains an explicitly measured zero in the requested range', () => {
    const result = accountStatisticsPeriod(snapshot([{ date: '2026-09-18', tokens: 0 }], summary), 'all', now, { startDate: '2026-09-18', endDate: '' });
    expect(result).toMatchObject({ totalTokens: 0, totalSource: 'daily', peakDailyTokens: 0, peakSource: 'daily' });
  });

  it.each([
    { startDate: '2026-09-18', endDate: '2026-09-17' },
    { startDate: '2026-09-19', endDate: '' },
    { startDate: '', endDate: '2026-09-19' },
    { startDate: '2026-02-29', endDate: '' },
  ])('rejects invalid ranges without falling back to lifetime metrics (%j)', (range) => {
    expect(accountStatisticsPeriod(snapshot(records, summary), 'all', now, range)).toMatchObject({
      dailyUsage: null, totalTokens: null, totalSource: null, peakDailyTokens: null, peakSource: null,
    });
  });

  it.each(periods.filter((period) => period !== 'all'))('ignores even an invalid saved custom range for %s', (period) => {
    const source = snapshot(records, summary);
    expect(accountStatisticsPeriod(source, period, now, { startDate: '2026-09-19', endDate: '2020-01-01' }))
      .toEqual(accountStatisticsPeriod(source, period, now));
  });

  it('restores authoritative lifetime summaries when both range inputs are cleared', () => {
    expect(accountStatisticsPeriod(snapshot(records, summary), 'all', now, { startDate: '', endDate: '' })).toMatchObject({
      startDate: null, endDate: '2026-09-18', totalTokens: 9999, totalSource: 'summary', peakDailyTokens: 8888, peakSource: 'summary',
    });
  });

  it.each(['Asia/Shanghai', 'America/Los_Angeles'])('preserves service calendar dates in %s', (timezone) => {
    vi.stubEnv('TZ', timezone);
    try {
      const result = accountStatisticsPeriod(snapshot(records, summary), 'all', new Date('2026-09-18T16:00:00Z'), { startDate: '2025-12-31', endDate: '2026-01-01' });
      expect(result.dailyUsage).toEqual([{ date: '2025-12-31', tokens: 10 }, { date: '2026-01-01', tokens: 20 }]);
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it('filters and sorts without mutating source data or the selected range', () => {
    const source = snapshot(records.map((record) => Object.freeze({ ...record })), summary);
    const before = structuredClone(source);
    Object.freeze(source.dailyUsage);
    Object.freeze(source.summary);
    Object.freeze(source);
    const range = Object.freeze({ startDate: '2025-12-31', endDate: '2026-01-01' });
    const timestamp = now.getTime();
    const result = accountStatisticsPeriod(source, 'all', now, range);
    expect(result.dailyUsage?.map(({ date }) => date)).toEqual(['2025-12-31', '2026-01-01']);
    expect(source).toEqual(before);
    expect(now.getTime()).toBe(timestamp);
  });
});
