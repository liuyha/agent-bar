import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AccountUsageSnapshot, TokenPeriod, WebUsageSnapshot } from '../types';
import { AccountStatisticsContent } from './AccountStatisticsContent';

const web: WebUsageSnapshot = {
  status: 'ready', message: null, account: 'web-account', creditsRemaining: null, codeReviewRemainingPercent: 0,
  usageUnit: 'requests', usageBreakdown: [{ date: '2026-09-12', amounts: [{ service: 'web-service', amount: 1.25 }] }],
  creditEvents: [{ date: '2026-09-12', service: 'correction', credits: -2.5 }], updatedAt: null,
};
const statistics: AccountUsageSnapshot = {
  source: 'oauth', status: 'ready', account: 'server-account', accountId: 'account-id', message: null,
  summary: { lifetimeTokens: 1000, peakDailyTokens: null, longestRunningTurnSec: 125, currentStreakDays: 0, longestStreakDays: null },
  dailyUsage: [{ date: '2026-09-12', tokens: 1000 }, { date: '2026-09-14', tokens: 0 }],
  serviceUpdatedAt: null, updatedAt: '2026-09-17T00:00:00Z', web,
};

function render(data: AccountUsageSnapshot | null = statistics, webEnabled = false, error: string | null = null, selectedPeriod: TokenPeriod['period'] | undefined = 'all', loading = false, refreshing = loading) {
  return renderToStaticMarkup(createElement(AccountStatisticsContent, { statistics: data, loading, refreshing, error, selectedPeriod, onPeriodChange: () => {}, webEnabled, onRetry: () => {}, onOpenWeb: () => {} }));
}

describe('account statistics content', () => {
  beforeEach(() => { vi.useFakeTimers(); vi.setSystemTime(new Date(2026, 8, 17, 12)); });
  afterEach(() => { vi.useRealTimers(); });

  it('defaults to today and shares the five-option selector with local statistics', () => {
    const html = renderToStaticMarkup(createElement(AccountStatisticsContent, { statistics, loading: false, refreshing: false, error: null, webEnabled: false, onPeriodChange: () => {}, onRetry: () => {}, onOpenWeb: () => {} }));
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
    const html = render({ ...statistics, summary: { ...statistics.summary, lifetimeTokens: 999, peakDailyTokens: 500 }, dailyUsage }, false, null, period);
    expect(html).toContain(`aria-label="${label}服务端 Token 统计"`);
    expect(html.match(/<input[^>]*checked=""[^>]*>/)?.[0]).toContain(`value="${period}"`);
    expect(html).toContain(`<dd title="${total}">${total}</dd>`);
    expect(html).toContain(`<dd title="${period === 'all' ? 500 : 64}">${period === 'all' ? 500 : 64}</dd>`);
    expect([...html.matchAll(/<time>([^<]+)<\/time>/g)].map((match) => match[1])).toEqual(dates);
    expect(html).toContain('账号活动概览（不随时段切换）');
    expect(html).toContain('2 分 5 秒');
    expect(html).toContain(period === 'all' ? '每日记录可能仅覆盖部分历史' : '可能不完整');
  });

  it('distinguishes no returned dates from a genuine zero and keeps today independent of stale service updates', () => {
    const missing = render({ ...statistics, serviceUpdatedAt: '2026-09-14' }, false, null, 'day');
    expect(missing).toContain('服务端尚未返回今日日期范围内的每日记录');
    expect(missing).toContain('<dt>Token 合计（已返回）</dt><dd>—</dd>');
    expect(missing).not.toContain('<time>2026-09-14</time>');
    const zero = render({ ...statistics, dailyUsage: [{ date: '2026-09-17', tokens: 0 }] }, false, null, 'day');
    expect(zero).toContain('<dt>Token 合计（已返回）</dt><dd title="0">0</dd>');
  });

  it('keeps unavailable lifetime totals unknown even when returned daily records have values', () => {
    const html = render({ ...statistics, summary: { ...statistics.summary, lifetimeTokens: null } });
    expect(html).toContain('<dt>累计 Token</dt><dd>—</dd>');
    expect(html).toContain('<time>2026-09-12</time>');
    expect(html).toContain('不能据此还原全部用量');
  });

  it('hides period totals only during manual collection and when there is no usable result', () => {
    for (const html of [render(statistics, false, null, 'week', true), render(null, false, '读取失败')]) {
      expect(html).not.toContain('account-period-metrics');
      expect(html).not.toContain('role="radiogroup"');
    }
  });

  it('keeps cached totals and webpage data during silent refresh while rotating an available refresh button', () => {
    const html = render(statistics, true, null, 'all', false, true);
    expect(html).toContain('account-period-metrics');
    expect(html).toContain('<dd title="1,000">1K</dd>');
    expect(html).toContain('web-account');
    expect(html).not.toContain('正在读取服务端统计');
    expect(html).not.toContain('class="statistics-state"');
    expect(html).toMatch(/<button[^>]*aria-label="刷新服务端统计"[^>]*aria-busy="true"[^>]*>[^]*?<svg[^>]*class="[^"]*spin/);
    // Tailwind's disabled: utility classes do not make the button disabled.
    expect(html.match(/<button[^>]*aria-label="刷新服务端统计"[^>]*>/)?.[0]).not.toMatch(/\sdisabled(?:=|\s|>)/);
  });

  it('shows panel loading and hides cached webpage data for a manual refresh', () => {
    const html = render(statistics, true, null, 'all', true, true);
    expect(html).toContain('正在读取服务端统计');
    expect(html).not.toContain('account-period-metrics');
    expect(html).not.toContain('web-account');
    expect(html.match(/<button[^>]*aria-label="刷新服务端统计"[^>]*>/)?.[0]).toMatch(/\sdisabled(?:=|\s|>)/);
  });

  it('fetches without a panel loading screen when the automatic first read has no cache', () => {
    const html = render(null, false, null, 'all', false, true);
    expect(html).toContain('暂无本地缓存，正在后台获取服务端统计');
    expect(html).not.toContain('正在读取服务端统计');
    expect(html).not.toContain('class="statistics-state"');
    expect(html).not.toContain('account-period-metrics');
  });

  it('keeps validated cached values, their source and original timestamps when background refresh fails', () => {
    const html = render(statistics, false, '网络暂不可用');
    expect(html).toContain('更新失败：网络暂不可用 当前显示本地缓存');
    expect(html).toContain('role="alert"');
    expect(html).toContain('实际来源：服务端');
    expect(html).toContain('<dd title="1,000">1K</dd>');
    expect(html).not.toContain('正在读取服务端统计');
  });
  it.each(['oauth', 'pat', 'cli'] as const)('shows the server without exposing its %s authentication strategy', (source) => {
    const html = render({ ...statistics, source });
    expect(html).toContain('实际来源：服务端');
    for (const strategy of ['OAuth', 'PAT', 'CLI']) expect(html).not.toContain(strategy);
  });
  it('preserves date-only service updates without fabricating a time of day', () => {
    const html = render({ ...statistics, serviceUpdatedAt: '2026-09-16' });
    expect(html).toContain('服务端更新于 2026-09-16</div>');
    expect(html).not.toContain('服务端更新于 2026/9/16');
  });
  it.each(['unavailable', 'error'] as const)('does not label the requested source as collected for %s responses', (status) => {
    const html = render({ ...statistics, status });
    expect(html).toContain('实际来源：—');
    expect(html).not.toContain('实际来源：服务端');
  });
  it('requires a collected timestamp before identifying an actual source', () => {
    expect(render({ ...statistics, updatedAt: null })).toContain('实际来源：—');
    expect(render({ ...statistics, updatedAt: 'invalid' })).toContain('实际来源：—');
    expect(render()).toContain('实际来源：服务端');
  });
  it('keeps service dates without filling gaps and does not fabricate local metrics', () => {
    const html = render();
    expect(html).toContain('实际来源：服务端');
    expect(html).toContain('server-account');
    expect(html).toContain('account-id');
    expect(html).toContain('2026-09-12');
    expect(html).toContain('2026-09-14');
    expect(html).not.toContain('2026-09-13');
    expect(html).toContain('2 分 5 秒');
    expect(html).toContain('0 天');
    expect(html).toContain('<dt>单日峰值 Token</dt><dd>—</dd>');
    expect(html).toContain('服务端更新于 —');
    for (const label of ['约等金额', '请求数', '会话轮次', '输入（含缓存）', '缓存读取']) expect(html).not.toContain(label);
  });
  it('hides all webpage data and controls when supplemental collection is off', () => {
    const html = render(statistics, false);
    for (const text of ['web-account', 'web-service', 'correction', '连接用量网页', '剩余 Credits']) expect(html).not.toContain(text);
  });
  it('preserves webpage units, fractional usage, negative credit events and genuine zeros', () => {
    const html = render(statistics, true);
    expect(html).toContain('网页用量明细 · requests');
    expect(html).toContain('1.25<small> requests</small>');
    expect(html).toContain('-2.5<small> Credits</small>');
    expect(html).toContain('<dt>剩余 Credits</dt><dd>—</dd>');
    expect(html).toContain('<dt>代码审查剩余额度</dt><dd>0%</dd>');
    expect(html).toContain('返回后刷新');
  });
  it('does not assume Tokens when webpage units are missing', () => {
    const html = render({ ...statistics, web: { ...web, usageUnit: null } }, true);
    expect(html).toContain('网页用量明细 · 单位未提供');
    expect(html).toContain('<dd>1.25</dd>');
    expect(html).not.toContain('1.25<small> Token');
  });
  it('handles missing and empty service fields separately without inventing zero records', () => {
    const unknown = render({ ...statistics, dailyUsage: null, web: { ...web, usageBreakdown: null, creditEvents: null } }, true);
    expect(unknown).toContain('服务端未提供每日 Token');
    expect(unknown).toContain('网页未提供用量明细');
    expect(unknown).toContain('网页未提供 Credits 记录');
    const empty = render({ ...statistics, dailyUsage: [], web: { ...web, usageBreakdown: [], creditEvents: [] } }, true);
    expect(empty).toContain('服务端返回的每日记录为空');
    expect(empty).toContain('网页返回的用量明细为空');
    expect(empty).toContain('网页返回的 Credits 记录为空');
  });
  it('keeps webpage errors separate and does not render data from a failed result', () => {
    const failedWeb = render({ ...statistics, web: { ...web, status: 'error', message: '网页账号不匹配' } }, true);
    expect(failedWeb).toContain('网页账号不匹配');
    expect(failedWeb).not.toContain('web-account');
    expect(failedWeb).toContain('累计 Token');
    const failedMain = render({ ...statistics, status: 'error', message: 'OAuth 认证失败', web: null });
    expect(failedMain).toContain('OAuth 认证失败');
    expect(failedMain).not.toContain('累计 Token');
    expect(failedMain).not.toContain('server-account');
    expect(render(null, true, '读取失败')).toContain('连接用量网页');
  });
});
