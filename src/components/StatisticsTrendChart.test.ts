import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { StatisticsTrendChart } from './StatisticsTrendChart';
import type { TrendData } from '../lib/statisticsTrend';

function render(data: TrendData) {
  return renderToStaticMarkup(createElement(StatisticsTrendChart, { data }));
}

describe('statistics trend chart', () => {
  it('breaks the line at missing records while retaining explicit zero values', () => {
    const html = render({ granularity: 'day', points: [100, null, 0, 200].map((tokens, index) => ({ key: String(index), label: `2026-09-${10 + index}`, tokens })) });
    const path = html.match(/class="statistics-chart-line" d="([^"]+)"/)?.[1];
    expect(path?.match(/M/g)).toHaveLength(2);
    expect(path?.match(/L/g)).toHaveLength(1);
    expect(html.match(/class="statistics-chart-dot"/g)).toHaveLength(3);
    expect(html).toContain('每日 Token 折线图');
    expect(html).toContain('aria-pressed="true">折线');
    expect(html).toContain('aria-pressed="false">柱状');
  });

  it('shows a genuine all-zero series instead of an empty state', () => {
    const html = render({ granularity: 'hour', points: [{ key: 'zero', label: '2026-09-20 00:00', tokens: 0 }] });
    expect(html).toContain('每小时 Token 折线图');
    expect(html).toContain('00:00');
    expect(html).not.toContain('statistics-chart-empty');
    expect(html).not.toMatch(/NaN|Infinity/);
  });

  it('explains unavailable hourly records without drawing a fabricated series', () => {
    const html = render({ granularity: 'hour', points: [], message: '服务端未提供小时记录。' });
    expect(html).toContain('按小时');
    expect(html).toContain('服务端未提供小时记录');
    expect(html).not.toContain('<svg');
  });

  it('keeps each day in a long history and enables horizontal scrolling', () => {
    const html = render({ granularity: 'day', points: Array.from({ length: 365 }, (_, index) => ({ key: String(index), label: '2026-01-01', tokens: index })) });
    expect(html.match(/class="statistics-chart-dot"/g)).toHaveLength(365);
    expect(html).toContain('min-width:3710px');
    expect(html).toContain('可横向滚动');
  });
});
