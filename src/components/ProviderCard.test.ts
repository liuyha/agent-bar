import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { ProviderUsage } from '../types';
import { ProviderCard } from './ProviderCard';

const account: ProviderUsage = {
  id: 'codex', name: 'Codex', source: 'local', status: 'ready', plan: 'Plus',
  account: 'account@example.com', message: null, updatedAt: '2026-09-16T00:00:00Z',
  windows: [{ label: '5 小时用量', usedPercent: 0, resetsAt: null }],
};

function render(overrides: Partial<ProviderUsage> = {}): string {
  return renderToStaticMarkup(createElement(ProviderCard, { provider: { ...account, ...overrides }, now: Date.parse('2026-09-16T00:00:00Z') }));
}

describe('account usage states', () => {
  it('keeps refresh progress and failures inside the targeted service card', () => {
    const html = renderToStaticMarkup(createElement(ProviderCard, { provider: account, now: 0, onRefresh: () => {}, refreshing: true, refreshError: '本卡片刷新失败' }));
    expect(html).toContain('aria-label="刷新 Codex 用量"');
    expect(html).toContain('disabled=""');
    expect(html).toContain('正在刷新 Codex');
    expect(html).toContain('本卡片刷新失败');
    const other = renderToStaticMarkup(createElement(ProviderCard, { provider: { ...account, id: 'claude', name: 'Claude' }, now: 0, onRefresh: () => {} }));
    expect(other).toContain('aria-label="刷新 Claude 用量"');
    expect(other).not.toContain('disabled=""');
    expect(other).not.toContain('本卡片刷新失败');
  });

  it('shows account, plan and full remaining quota for zero usage while leaving unknown reset time unknown', () => {
    const html = render();
    expect(html).toContain('account@example.com');
    expect(html).toContain('Plus');
    expect(html).toContain('已连接');
    expect(html).toContain('100% 剩余');
    expect(html).toContain('aria-valuenow="100"');
    expect(html).toContain('width:100%');
    expect(html).toContain('时间未知');
    expect(html).not.toContain('后重置');
  });

  it.each([
    { used: 38, remaining: 62, warning: false },
    { used: 80, remaining: 20, warning: true },
    { used: 120, remaining: 0, warning: true },
    { used: -10, remaining: 100, warning: false },
  ])('shows $remaining% remaining for $used% used and preserves low-quota warnings', ({ used, remaining, warning }) => {
    const html = render({ windows: [{ label: '每周用量', usedPercent: used, resetsAt: '2026-09-18T20:00:00Z' }] });
    expect(html).toContain(`${remaining}% 剩余`);
    expect(html).toContain(`aria-valuenow="${remaining}"`);
    expect(html).toContain(`width:${remaining}%`);
    expect(html).toContain('aria-label="Codex 每周用量剩余"');
    expect(html.includes('progress-high')).toBe(warning);
    expect(html).toContain('2 天 20 小时后重置');
  });

  it('shows a missing account explanation without displaying a zero-usage meter', () => {
    const html = render({ status: 'unavailable', account: null, plan: '', message: '未找到已登录的 Codex 账号。', windows: [] });
    expect(html).toContain('未找到已登录的 Codex 账号。');
    expect(html).toContain('待获取');
    expect(html).not.toContain('role="progressbar"');
    expect(html).not.toContain('Plus');
    expect(html).not.toContain('account@example.com');
  });

  it('keeps the initial account-reading state visible', () => {
    const html = render({ status: 'unavailable', message: '正在读取本机账号', windows: [] });
    expect(html).toContain('正在读取本机账号');
    expect(html).not.toContain('读取失败');
    expect(html).not.toContain('role="progressbar"');
  });

  it('shows provider errors without retaining a misleading usage meter', () => {
    const html = render({ status: 'error', message: '登录已过期，请重新登录 Codex 后刷新。' });
    expect(html).toContain('读取失败');
    expect(html).toContain('登录已过期');
    expect(html).toContain('role="alert"');
    expect(html).not.toContain('role="progressbar"');
  });

  it('does not treat missing or malformed windows as zero usage', () => {
    const empty = render({ windows: [] });
    const invalid = render({ windows: [{ label: '5 小时用量', usedPercent: Number.NaN, resetsAt: null }] });
    for (const html of [empty, invalid]) {
      expect(html).toContain('当前账号未返回可用的用量信息');
      expect(html).not.toContain('role="progressbar"');
    }
  });

  it('renders every returned usage bucket without assuming two windows', () => {
    const html = render({ windows: [
      { label: '每周用量', usedPercent: 12, resetsAt: null },
      { label: 'Codex Spark · 5 小时用量', usedPercent: 4, resetsAt: null },
      { label: 'Codex Spark · 每周用量', usedPercent: 7, resetsAt: null },
    ] });
    expect(html.match(/role="progressbar"/g)).toHaveLength(3);
    expect(html).toContain('Codex Spark · 5 小时用量');
    expect(html).toContain('Codex Spark · 每周用量');
  });
});
