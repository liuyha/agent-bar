import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AccountUsageSnapshot, TokenPeriod } from '../types';
import { AccountStatisticsContent } from './AccountStatisticsContent';

const statistics: AccountUsageSnapshot = {
  source: 'oauth', status: 'ready', account: 'server-account', accountId: 'account-id', message: null,
  summary: { lifetimeTokens: 1000, peakDailyTokens: null, longestRunningTurnSec: 125, currentStreakDays: 0, longestStreakDays: null },
  dailyUsage: [{ date: '2026-09-12', tokens: 1000 }, { date: '2026-09-14', tokens: 0 }],
  serviceUpdatedAt: null, updatedAt: '2026-09-17T00:00:00Z',
};

function render(data: AccountUsageSnapshot | null = statistics, error: string | null = null, selectedPeriod: TokenPeriod['period'] | undefined = 'all', loading = false, refreshing = loading) {
  return renderToStaticMarkup(createElement(AccountStatisticsContent, { statistics: data, loading, refreshing, error, selectedPeriod, onPeriodChange: () => {}, onRetry: () => {} }));
}

describe('account statistics content', () => {
  beforeEach(() => { vi.useFakeTimers(); vi.setSystemTime(new Date(2026, 8, 17, 12)); });
  afterEach(() => { vi.useRealTimers(); });

  it('defaults to today and shares the five-option selector with local statistics', () => {
    const html = renderToStaticMarkup(createElement(AccountStatisticsContent, { statistics, loading: false, refreshing: false, error: null, onPeriodChange: () => {}, onRetry: () => {} }));
    expect(html).toContain('role="radiogroup" aria-label="统计时段"');
    expect([...html.matchAll(/<input[^>]*value="([^"]+)"/g)].map((match) => match[1])).toEqual(['day', 'week', 'month', 'year', 'all']);
    expect(html.match(/<input[^>]*checked=""[^>]*>/g)).toHaveLength(1);
    expect(html.match(/<input[^>]*checked=""[^>]*>/)?.[0]).toContain('value="day"');
    for (const label of ['今日', '本周', '本月', '本年', '全部']) expect(html).toContain(label);
    expect(html).toContain('今日服务端 Token 统计');
  });

  it.each([
    ['day', '今日', 64, ['2026-09-17']],
    ['week', '本周', 96, ['2026-09-14', '2026-09-17']],
    ['month', '本月', 120, ['2026-09-01', '2026-09-13', '2026-09-14', '2026-09-17']],
    ['year', '本年', 126, ['2026-01-01', '2026-08-31', '2026-09-01', '2026-09-13', '2026-09-14', '2026-09-17']],
    ['all', '全部', 999, ['2025-12-31', '2026-01-01', '2026-08-31', '2026-09-01', '2026-09-13', '2026-09-14', '2026-09-17']],
  ] as const)('changes totals, peaks and daily rows together for %s', (period, label, total, dates) => {
    const dailyUsage = ['2025-12-31', '2026-01-01', '2026-08-31', '2026-09-01', '2026-09-13', '2026-09-14', '2026-09-17', '2026-09-18'].map((date, index) => ({ date, tokens: 2 ** index }));
    const html = render({ ...statistics, summary: { ...statistics.summary, lifetimeTokens: 999, peakDailyTokens: 500 }, dailyUsage }, null, period);
    expect(html).toContain(`aria-label="${label}服务端 Token 统计"`);
    expect(html.match(/<input[^>]*checked=""[^>]*>/)?.[0]).toContain(`value="${period}"`);
    expect(html).toContain(`<dd title="${total}">${total}</dd>`);
    expect(html).toContain(`<dd title="${period === 'all' ? 500 : 64}">${period === 'all' ? 500 : 64}</dd>`);
    expect([...html.matchAll(/<time>([^<]+)<\/time>/g)].map((match) => match[1])).toEqual(dates);
    expect(html).toContain('<summary>账号活动概览</summary>');
    expect(html).toContain('2 分 5 秒');
    expect(html).not.toContain(period === 'all' ? '每日记录可能仅覆盖部分历史' : '可能不完整');
  });

  it('distinguishes no returned dates from a genuine zero and keeps today independent of stale service updates', () => {
    const missing = render({ ...statistics, serviceUpdatedAt: '2026-09-14' }, null, 'day');
    expect(missing).toContain('服务端尚未返回今日日期范围内的每日记录');
    expect(missing).toContain('<dt>Token 合计</dt><dd>—</dd>');
    expect(missing).not.toContain('<time>2026-09-14</time>');
    const zero = render({ ...statistics, dailyUsage: [{ date: '2026-09-17', tokens: 0 }] }, null, 'day');
    expect(zero).toContain('<dt>Token 合计</dt><dd title="0">0</dd>');
  });

  it('keeps unavailable lifetime totals unknown even when returned daily records have values', () => {
    const html = render({ ...statistics, summary: { ...statistics.summary, lifetimeTokens: null } });
    expect(html).toContain('<dt>累计 Token</dt><dd>—</dd>');
    expect(html).toContain('<time>2026-09-12</time>');
    expect(html).not.toContain('不能据此还原全部用量');
  });

  it('hides period totals only during manual collection and when there is no usable result', () => {
    for (const html of [render(statistics, null, 'week', true), render(null, '读取失败')]) {
      expect(html).not.toContain('account-period-metrics');
      expect(html).not.toContain('role="radiogroup"');
    }
  });

  it('keeps cached totals during silent refresh', () => {
    const html = render(statistics, null, 'all', false, true);
    expect(html).toContain('account-period-metrics');
    expect(html).toContain('<dd title="1,000">1K</dd>');
    expect(html).not.toContain('正在读取服务端统计');
    expect(html).not.toContain('class="statistics-state"');
  });

  it('shows panel loading and hides cached totals for a manual refresh', () => {
    const html = render(statistics, null, 'all', true, true);
    expect(html).toContain('正在读取服务端统计');
    expect(html).not.toContain('account-period-metrics');
  });

  it('fetches without a panel loading screen when the automatic first read has no cache', () => {
    const html = render(null, null, 'all', false, true);
    expect(html).toContain('暂无本地缓存，正在后台获取服务端统计');
    expect(html).not.toContain('正在读取服务端统计');
    expect(html).not.toContain('class="statistics-state"');
    expect(html).not.toContain('account-period-metrics');
  });

  it('keeps validated cached values when background refresh fails', () => {
    const html = render(statistics, '网络暂不可用');
    expect(html).toContain('更新失败：网络暂不可用 当前显示本地缓存');
    expect(html).toContain('role="alert"');
    expect(html).not.toContain('实际来源：');
    expect(html).toContain('<dd title="1,000">1K</dd>');
    expect(html).not.toContain('正在读取服务端统计');
  });
  it.each(['oauth', 'pat', 'cli'] as const)('omits source details and its %s authentication strategy', (source) => {
    const html = render({ ...statistics, source });
    expect(html).not.toContain('实际来源：');
    for (const strategy of ['OAuth', 'PAT', 'CLI']) expect(html).not.toContain(strategy);
  });
  it('preserves date-only service updates without fabricating a time of day', () => {
    const html = render({ ...statistics, serviceUpdatedAt: '2026-09-16' });
    expect(html).toContain('服务端更新于 2026-09-16</div>');
    expect(html).not.toContain('服务端更新于 2026/9/16');
  });
  it('keeps service dates without filling gaps and does not fabricate local metrics', () => {
    const html = render();
    expect(html).not.toContain('实际来源：');
    expect(html).not.toContain('server-account');
    expect(html).not.toContain('account-id');
    expect(html).toContain('2026-09-12');
    expect(html).toContain('2026-09-14');
    expect(html).not.toContain('2026-09-13');
    expect(html).toContain('2 分 5 秒');
    expect(html).toContain('0 天');
    expect(html).toContain('<dt>单日峰值 Token</dt><dd>—</dd>');
    expect(html).toContain('服务端更新于 —');
    for (const label of ['约等金额', '请求数', '会话轮次', '输入（含缓存）', '缓存读取']) expect(html).not.toContain(label);
  });
  it('handles missing and empty service fields separately without inventing zero records', () => {
    const unknown = render({ ...statistics, dailyUsage: null });
    expect(unknown).toContain('服务端未提供每日 Token 用量记录');
    const empty = render({ ...statistics, dailyUsage: [] });
    expect(empty).toContain('服务端返回的每日记录为空');
  });
  it('does not render data from a failed result', () => {
    const failed = render({ ...statistics, status: 'error', message: 'OAuth 认证失败' });
    expect(failed).toContain('OAuth 认证失败');
    expect(failed).not.toContain('累计 Token');
    expect(failed).not.toContain('server-account');
  });
});
