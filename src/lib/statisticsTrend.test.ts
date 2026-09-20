import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AccountUsageSnapshot, TokenPeriod, TokenStatistics } from '../types';
import { accountStatisticsTrend, localStatisticsTrend } from './statisticsTrend';

function bucket(startAt: string, totalTokens: number, period: TokenPeriod['period'] = 'day', endAt = startAt): TokenPeriod {
  return {
    period, startAt: new Date(startAt).toISOString(), endAt: new Date(endAt).toISOString(),
    totalTokens, inputTokens: totalTokens, cachedInputTokens: 0, cacheWriteTokens: 0, outputTokens: 0,
    estimatedCostUsd: 0, unpricedTokens: 0, requestCount: 0, conversationTurns: 0,
  };
}

const now = new Date('2026-09-20T10:30:00');
function local(overrides: Partial<TokenStatistics> = {}): TokenStatistics {
  return {
    status: 'ready', message: null, updatedAt: now.toISOString(),
    periods: [
      bucket('2026-09-20T00:00:00', 40, 'day', '2026-09-20T10:30:00'),
      bucket('2026-09-14T00:00:00', 40, 'week', '2026-09-20T10:30:00'),
      bucket('2025-12-31T10:00:00', 40, 'all', '2026-09-20T10:30:00'),
    ],
    hourlyPeriods: [bucket('2026-09-19T23:00:00', 70), bucket('2026-09-20T01:00:00', 40)],
    dailyPeriods: [bucket('2026-09-18T00:00:00', 40)], ...overrides,
  };
}

function account(dailyUsage: AccountUsageSnapshot['dailyUsage']): AccountUsageSnapshot {
  return {
    source: 'oauth', status: 'ready', message: null, account: null, accountId: null,
    summary: { lifetimeTokens: 999, peakDailyTokens: 999, longestRunningTurnSec: null, currentStreakDays: null, longestStreakDays: null },
    dailyUsage, updatedAt: now.toISOString(), serviceUpdatedAt: null,
  };
}

afterEach(() => { vi.useRealTimers(); vi.unstubAllEnvs(); });

function freezeNow() {
  vi.useFakeTimers();
  vi.setSystemTime(now);
}

describe('local usage trends', () => {
  it('fills elapsed hours, includes the current partial hour and excludes yesterday and future hours', () => {
    freezeNow();
    const result = localStatisticsTrend(local(), 'day');
    expect(result.granularity).toBe('hour');
    expect(result.points).toHaveLength(11);
    expect(result.points.map((point) => point.tokens)).toEqual([0, 40, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    expect(result.points[10].label).toContain('2026-09-20 10:00');
  });

  it('does not invent observations after an older snapshot', () => {
    freezeNow();
    const result = localStatisticsTrend(local({ updatedAt: new Date('2026-09-20T08:15:00').toISOString() }), 'day');
    expect(result.points).toHaveLength(9);
    expect(result.points.at(-1)?.label).toContain('08:00');
  });

  it('uses calendar days for other periods, filling absent local events with measured zero', () => {
    freezeNow();
    const result = localStatisticsTrend(local(), 'week');
    expect(result.granularity).toBe('day');
    expect(result.points.map((point) => point.tokens)).toEqual([0, 0, 0, 0, 40, 0, 0]);
    expect(result.points[0].key).toBe('2026-09-14');
    expect(result.points.at(-1)?.key).toBe('2026-09-20');
  });

  it('keeps both boundaries across years and permits open date boundaries', () => {
    freezeNow();
    const source = local({ dailyPeriods: [bucket('2025-12-31T00:00:00', 10), bucket('2026-01-01T00:00:00', 20)] });
    expect(localStatisticsTrend(source, 'all', { startDate: '', endDate: '2026-01-01' }).points).toEqual([
      { key: '2025-12-31', label: '2025-12-31', tokens: 10 },
      { key: '2026-01-01', label: '2026-01-01', tokens: 20 },
    ]);
    expect(localStatisticsTrend(source, 'all', { startDate: '2026-09-19', endDate: '' }).points.map((point) => point.tokens)).toEqual([0, 0]);
  });

  it.each([null, undefined])('requests refresh for absent old-cache details (%s)', (missing) => {
    freezeNow();
    expect(localStatisticsTrend(local({ hourlyPeriods: missing }), 'day')).toMatchObject({ points: [], message: expect.stringContaining('刷新') });
    expect(localStatisticsTrend(local({ dailyPeriods: missing }), 'week')).toMatchObject({ points: [], message: expect.stringContaining('刷新') });
    expect(localStatisticsTrend(local({ hourlyPeriods: [] }), 'day').points.every((point) => point.tokens === 0)).toBe(true);
  });

  it('rejects invalid or future date ranges and unavailable statistics', () => {
    freezeNow();
    for (const range of [
      { startDate: '2026-09-20', endDate: '2026-09-19' },
      { startDate: '', endDate: '2026-09-21' },
      { startDate: '2026-02-30', endDate: '' },
    ]) expect(localStatisticsTrend(local(), 'all', range).points).toEqual([]);
    expect(localStatisticsTrend(local({ status: 'error' }), 'day').points).toEqual([]);
    expect(localStatisticsTrend(null, 'day').points).toEqual([]);
  });

  it('preserves repeated DST hours as distinct points and skips the missing spring hour', () => {
    vi.stubEnv('TZ', 'America/New_York');
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-11-01T08:00:00Z'));
    const source = local({
      updatedAt: '2026-11-01T08:00:00Z',
      periods: [bucket('2026-11-01T04:00:00Z', 30, 'day', '2026-11-01T08:00:00Z')],
      hourlyPeriods: [bucket('2026-11-01T05:00:00Z', 10), bucket('2026-11-01T06:00:00Z', 20)],
    });
    const repeated = localStatisticsTrend(source, 'day').points.filter((point) => point.label.includes('01:00'));
    expect(repeated.map((point) => point.tokens)).toEqual([10, 20]);
    expect(repeated[0].label).not.toBe(repeated[1].label);
    vi.setSystemTime(new Date('2026-03-08T08:00:00Z'));
    const spring = localStatisticsTrend(local({
      updatedAt: '2026-03-08T08:00:00Z', hourlyPeriods: [],
      periods: [bucket('2026-03-08T05:00:00Z', 0, 'day', '2026-03-08T08:00:00Z')],
    }), 'day');
    expect(spring.points.map((point) => point.label.slice(11, 16))).toEqual(['00:00', '01:00', '03:00', '04:00']);
  });
});

describe('account usage trends', () => {
  it('explains unavailable hourly data instead of spreading the daily total into invented hours', () => {
    expect(accountStatisticsTrend(account([{ date: '2026-09-19', tokens: 24 }]), 'day', undefined, now))
      .toMatchObject({ granularity: 'hour', points: [], message: expect.stringContaining('暂未提供小时记录') });
  });

  it('preserves actual zeroes, missing dates, chronological order and future exclusion', () => {
    const source = account([{ date: '2026-09-20', tokens: 30 }, { date: '2026-09-18', tokens: 0 }, { date: '2026-09-21', tokens: 500 }]);
    const before = structuredClone(source);
    const result = accountStatisticsTrend(source, 'week', undefined, now);
    expect(result.points.map((point) => point.tokens)).toEqual([null, null, null, null, 0, null, 30]);
    expect(result.points.at(-1)?.key).toBe('2026-09-20');
    expect(source).toEqual(before);
  });

  it.each([null, []])('keeps unavailable daily history unknown (%j)', (records) => {
    expect(accountStatisticsTrend(account(records), 'week', undefined, now).points.map((point) => point.tokens)).toEqual(Array(7).fill(null));
    expect(accountStatisticsTrend(account(records), 'all', undefined, now).points).toEqual([]);
  });

  it('uses earliest available record for all-time and keeps a multi-year series daily', () => {
    const result = accountStatisticsTrend(account([{ date: '2024-01-01', tokens: 5 }]), 'all', undefined, now);
    expect(result.granularity).toBe('day');
    expect(result.points.length).toBeGreaterThan(900);
    expect(result.points[0]).toEqual({ key: '2024-01-01', label: '2024-01-01', tokens: 5 });
    expect(result.points[1].key).toBe('2024-01-02');
  });

  it('limits an explicitly selected daily range and rejects invalid or excessive ranges', () => {
    const source = account([{ date: '2025-12-31', tokens: 5 }, { date: '2026-01-02', tokens: 10 }]);
    expect(accountStatisticsTrend(source, 'all', { startDate: '2025-12-31', endDate: '2026-01-01' }, now).points.map((point) => point.tokens)).toEqual([5, null]);
    expect(accountStatisticsTrend(source, 'all', { startDate: '2026-09-20', endDate: '2026-09-19' }, now).points).toEqual([]);
    expect(accountStatisticsTrend(source, 'all', { startDate: '0001-01-01', endDate: '' }, now)).toMatchObject({ points: [], message: expect.stringContaining('范围过长') });
  });
});
