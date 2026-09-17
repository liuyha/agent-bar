import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { StatisticsContent, TokenStatisticsPanel } from './TokenStatisticsPanel';
import { createAccountStatisticsStore } from '../lib/accountStatistics';
import { defaultSettings } from '../lib/settings';
import type { TokenPeriod, TokenStatistics } from '../types';

const statistics: TokenStatistics = {
  status: 'ready', message: null, updatedAt: '2026-09-16T10:00:00Z',
  activity: { longestRunningTurnSec: 3725, currentStreakDays: 7, longestStreakDays: 30 },
  periods: ['day', 'week', 'month', 'year', 'all'].map((period, index) => ({
    period: period as TokenPeriod['period'], startAt: '2026-09-16T00:00:00Z', endAt: '2026-09-16T10:00:00Z',
    inputTokens: 1000 * (index + 1), cachedInputTokens: 250, cacheWriteTokens: 100, outputTokens: 500 * (index + 1), totalTokens: 1500 * (index + 1),
    estimatedCostUsd: 0.05 * (index + 1), unpricedTokens: 0, requestCount: 12 * (index + 1), conversationTurns: 3 * (index + 1),
  })),
};

function render(data: TokenStatistics | null = statistics, loading = false, error: string | null = null, selectedPeriod?: TokenPeriod['period']) {
  return renderToStaticMarkup(createElement(StatisticsContent, { statistics: data, loading, error, selectedPeriod, onPeriodChange: () => {}, onRetry: () => {} }));
}

describe('token statistics', () => {
  it.each(['background', 'manual'] as const)('preserves the server refresh button state in the header during %s refresh', async (mode) => {
    const store = createAccountStatisticsStore();
    const pending = store.refresh('auto', mode);
    try {
      const html = renderToStaticMarkup(createElement(TokenStatisticsPanel, {
        provider: 'codex', name: 'Codex', refreshKey: '', account: null,
        settings: { ...defaultSettings(), codexStatisticsSource: 'auto' },
        accountStatisticsStore: store, onPreferencesChange: async () => {},
      }));
      const header = html.match(/<div class="statistics-header">([^]*?)<\/button><\/div>/)?.[0];
      expect(header).toBeDefined();
      expect(header).toContain('统计来源');
      expect(header).toMatch(/<button[^>]*aria-label="刷新服务端统计"[^>]*aria-busy="true"[^>]*>[^]*?<svg[^>]*class="[^"]*spin/);
      const button = header?.match(/<button[^>]*aria-label="刷新服务端统计"[^>]*>/)?.[0];
      // Tailwind's disabled: utility classes do not make the button disabled.
      expect(button).toMatch(/\sdisabled(?:=|\s|>)/);
    } finally {
      store.cancel();
      await pending;
    }
  });

  it('compacts every token field with exact titles while leaving requests and turns unabridged', () => {
    const html = render({ ...statistics, periods: [{
      ...statistics.periods[0], totalTokens: 1234567890, inputTokens: 1234567,
      outputTokens: 999995, cachedInputTokens: 1200, cacheWriteTokens: 25000,
      unpricedTokens: 123456, requestCount: 1234567, conversationTurns: 2500,
    }] });
    expect(html).toContain('<strong title="1,234,567,890">1.23B</strong>');
    for (const [exact, compact] of [['1,234,567', '1.23M'], ['999,995', '1M'], ['1,200', '1.2K'], ['25,000', '25K']]) {
      expect(html).toContain(`<dd title="${exact}">${compact}</dd>`);
    }
    expect(html).toContain('<span title="123,456">123.46K</span> Token');
    expect(html).toContain('<dt>请求数</dt><dd>1,234,567<small>次</small>');
    expect(html).toContain('<dt>会话轮次</dt><dd>2,500<small>轮</small>');
  });

  it('defaults to today with a five-option period selector and one statistics section', () => {
    const html = render();
    for (const label of ['今日', '本周', '本月', '本年', '全部', '1.5K', '¥0.35', '请求数', '会话轮次', '缓存读取', '缓存写入']) expect(html).toContain(label);
    expect(html).toContain('role="radiogroup" aria-label="统计时段"');
    expect(html.match(/type="radio"/g)).toHaveLength(5);
    expect([...html.matchAll(/<input[^>]*value="([^"]+)"/g)].map((match) => match[1])).toEqual(['day', 'week', 'month', 'year', 'all']);
    expect(html.match(/<input[^>]*checked=""[^>]*>/g)).toHaveLength(1);
    expect(html.match(/<input[^>]*checked=""[^>]*>/)?.[0]).toContain('value="day"');
    expect(html.match(/class="statistics-period"/g)).toHaveLength(1);
    expect(html).toContain('aria-label="今日 Token 统计"');
    expect(html).not.toContain('aria-label="本周 Token 统计"');
    expect(html).not.toContain('aria-label="本月 Token 统计"');
    expect(html).not.toContain('3,000');
    expect(html).not.toContain('4,500');
  });

  it.each([
    ['day', '今日', '1.5K', '¥0.35', '12', '3'],
    ['week', '本周', '3K', '¥0.70', '24', '6'],
    ['month', '本月', '4.5K', '¥1.05', '36', '9'],
    ['year', '本年', '6K', '¥1.40', '48', '12'],
    ['all', '全部', '7.5K', '¥1.75', '60', '15'],
  ] as const)('renders only the selected %s period with its own totals and counts', (period, label, tokens, cost, requests, turns) => {
    const html = render(statistics, false, null, period);
    expect(html.match(/class="statistics-period"/g)).toHaveLength(1);
    expect(html).toContain(`aria-label="${label} Token 统计"`);
    expect(html.match(/<input[^>]*checked=""[^>]*>/)?.[0]).toContain(`value="${period}"`);
    const exactTokens = statistics.periods.find((item) => item.period === period)!.totalTokens.toLocaleString('zh-CN');
    expect(html).toContain(`<strong title="${exactTokens}">${tokens}</strong>`);
    expect(html).toContain(cost);
    expect(html).toContain('约等金额 · 人民币');
    expect(html).toContain('按固定估算汇率 1 美元 ≈ 7 元人民币换算');
    expect(html).not.toContain('US$');
    expect(html).toContain('aria-label="本机活动概览"');
    expect(html).toContain('<dt>最长任务时长</dt><dd>1 小时 2 分</dd>');
    expect(html).toContain('<dt>当前连续活跃</dt><dd>7 天</dd>');
    expect(html).toContain('<dt>最长连续活跃</dt><dd>30 天</dd>');
    expect(html.indexOf('aria-label="本机活动概览"')).toBeLessThan(html.indexOf('role="radiogroup"'));
    expect(html).toContain(`<dt>请求数</dt><dd>${requests}<small>次</small>`);
    expect(html).toContain(`<dt>会话轮次</dt><dd>${turns}<small>轮</small>`);
  });

  it('keeps unknown prices and unknown counts distinct from true zero', () => {
    const unknown = render({ ...statistics, periods: [{ ...statistics.periods[0], estimatedCostUsd: null, unpricedTokens: 1500, requestCount: null, conversationTurns: null }] });
    expect(unknown).toContain('暂无法估算');
    expect(unknown).toContain('缺少可核实单价');
    expect(unknown).not.toContain('¥0.00');
    expect(unknown).toContain('—');
    const zero = render({ ...statistics, periods: [{ ...statistics.periods[0], totalTokens: 0, estimatedCostUsd: 0, requestCount: 0, conversationTurns: 0 }] });
    expect(zero).toContain('¥0.00');
    expect(zero).not.toContain('暂无法估算');
    const partial = render({ ...statistics, periods: [{ ...statistics.periods[0], unpricedTokens: 200 }] });
    expect(partial).toContain('（部分）');
    expect(partial).toContain('金额仅含已计价部分');
    expect(partial).toContain('¥0.35');
  });

  it('shows loading, unavailable and failed collection without fabricated totals', () => {
    for (const html of [render(null, true), render({ ...statistics, status: 'unavailable', message: '没有会话记录', periods: [], activity: null }), render(null, false, '读取失败')]) {
      expect(html).toContain('statistics-totals');
      expect(html).toContain('role="radiogroup"');
      expect(html).toContain('<span>Token 用量</span><strong>—</strong>');
      expect(html).not.toContain('¥0.00');
      expect(html).not.toContain('<strong>0</strong>');
      expect(html).not.toContain('class="statistics-state"');
      expect(html).toContain('<dt>最长任务时长</dt><dd>—</dd>');
      expect(html).toContain('<dt>当前连续活跃</dt><dd>—</dd>');
      expect(html).toContain('<dt>最长连续活跃</dt><dd>—</dd>');
    }
    expect(render(null, false, '读取失败')).toContain('role="alert"');
    expect(render(null, true)).toContain('正在统计本机会话');
  });

  it('keeps cached totals visible while refreshing and after a refresh failure', () => {
    for (const html of [render(statistics, true), render(statistics, false, '数据目录暂不可用')]) {
      expect(html).toContain('statistics-totals');
      expect(html).toContain('1.5K');
      expect(html).not.toContain('正在统计本机会话');
      expect(html).toContain('<dt>最长任务时长</dt><dd>1 小时 2 分</dd>');
      expect(html).toContain('<dt>当前连续活跃</dt><dd>7 天</dd>');
    }
    const failed = render(statistics, false, '数据目录暂不可用');
    expect(failed).toContain('role="alert"');
    expect(failed).toContain('当前显示上次统计结果');
    expect(failed).toContain('重新读取');
    expect(failed.indexOf('role="alert"')).toBeLessThan(failed.indexOf('role="radiogroup"'));
  });

  it('keeps failures in the top notice and retains unknown metrics for an unsuccessful result', () => {
    const html = render({ ...statistics, status: 'error', message: '连接失败' });
    expect(html.match(/连接失败/g)).toHaveLength(1);
    expect(html).toContain('statistics-totals');
    expect(html).toContain('<span>Token 用量</span><strong>—</strong>');
    expect(html).toContain('<dt>请求数</dt><dd>—');
    expect(html).toContain('<dt>输入（含缓存）</dt><dd>—</dd>');
    expect(html).not.toContain('1.5K');
    expect(html).toContain('<dt>最长任务时长</dt><dd>—</dd>');
    expect(html).toContain('<dt>最长连续活跃</dt><dd>—</dd>');
    expect(html).not.toContain('当前显示上次统计结果');
    expect(html.indexOf('role="alert"')).toBeLessThan(html.indexOf('role="radiogroup"'));
  });

  it('disables error retries while a refresh is running and keeps cached totals', () => {
    const html = render(statistics, true, '连接失败');
    expect(html).toMatch(/<button[^>]*\sdisabled=""[^>]*aria-busy="true"/);
    expect(html).toContain('读取中…');
    expect(html).toContain('1.5K');
  });

  it('keeps the last empty result visible during a background refresh', () => {
    const html = render({ ...statistics, status: 'unavailable', message: '没有会话记录', periods: [] }, true);
    expect(html).toContain('没有会话记录');
    expect(html).not.toContain('正在统计本机会话');
  });

  it('keeps activity unknown for older summaries while preserving recorded zero values', () => {
    for (const activity of [undefined, null]) {
      const html = render({ ...statistics, activity });
      expect(html).toContain('1.5K');
      expect(html).toContain('<dt>最长任务时长</dt><dd>—</dd>');
      expect(html).toContain('<dt>当前连续活跃</dt><dd>—</dd>');
      expect(html).toContain('<dt>最长连续活跃</dt><dd>—</dd>');
    }
    const html = render({ ...statistics, activity: { longestRunningTurnSec: null, currentStreakDays: 0, longestStreakDays: 3 } });
    expect(html).toContain('<dt>最长任务时长</dt><dd>—</dd>');
    expect(html).toContain('<dt>当前连续活跃</dt><dd>0 天</dd>');
    expect(html).toContain('<dt>最长连续活跃</dt><dd>3 天</dd>');
    expect(render({ ...statistics, activity: { longestRunningTurnSec: 0, currentStreakDays: 0, longestStreakDays: 0 } })).toContain('<dt>最长任务时长</dt><dd>0 秒</dd>');
  });

  it('shows known activity even when the local records contain no token usage', () => {
    const html = render({ ...statistics, status: 'unavailable', message: '未找到本机 Token 用量记录', periods: [] });
    expect(html).toContain('<dt>最长任务时长</dt><dd>1 小时 2 分</dd>');
    expect(html).toContain('<dt>当前连续活跃</dt><dd>7 天</dd>');
    expect(html).toContain('<span>Token 用量</span><strong>—</strong>');
  });

  it.each([['week', '本周'], ['year', '本年'], ['all', '全部']] as const)('does not substitute another period when %s is unavailable', (period, label) => {
    const html = render({ ...statistics, periods: [statistics.periods[0]] }, false, null, period);
    expect(html).toContain(`暂无${label}统计数据`);
    expect(html).not.toContain('statistics-totals');
  });
});
