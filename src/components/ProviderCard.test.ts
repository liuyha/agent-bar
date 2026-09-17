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
  it.each(['Pro 5x', 'Pro 20x'])('shows the precise %s subscription and an expandable reset count with expiry', (plan) => {
    const html = render({ plan, resetCredits: {
      remaining: 2, updatedAt: account.updatedAt, message: null,
      credits: [
        { id: 'one', remaining: 1, expiresAt: '2026-10-01T00:00:00Z' },
        { id: 'two', remaining: 1, expiresAt: '2026-11-01T00:00:00Z' },
      ],
    } });
    expect(html).toContain(plan);
    expect(html).toContain('<details class="reset-credits">');
    expect(html).toContain('Codex 重置剩余 2 次，查看重置次数列表');
    expect(html).toContain('重置次数列表');
    expect(html).toContain('2026/10/01');
    expect(html).toContain('2026/11/01');
    expect(html).toContain('<span>过期时间</span>');
  });

  it('distinguishes zero reset credits from missing data, and only shows them for connected Codex accounts', () => {
    expect(render({ resetCredits: { remaining: 0, credits: [], updatedAt: account.updatedAt, message: null } })).toContain('暂无可用重置次数');
    const unknown = render({ resetCredits: null });
    expect(unknown).toContain('重置剩余数量未知');
    expect(unknown).not.toContain('暂无可用重置次数');
    expect(render({ id: 'claude' })).not.toContain('reset-credits');
    expect(render({ status: 'unavailable' })).not.toContain('reset-credits');
  });

  it('keeps a known reset count when its expiry list is unavailable', () => {
    const html = render({ resetCredits: { remaining: 2, credits: null, updatedAt: null, message: '重置次数明细暂不可用' } });
    expect(html).toContain('Codex 重置剩余 2 次');
    expect(html).toContain('重置次数明细暂不可用');
    expect(html).not.toContain('暂无可用重置次数');
  });

  it('shows confirmed reset credits even when the account has no quota windows', () => {
    const html = render({ status: 'unavailable', windows: [], resetCredits: {
      remaining: 1, updatedAt: account.updatedAt, message: null,
      credits: [{ id: 'one', remaining: 1, expiresAt: '2026-10-01T00:00:00Z' }],
    } });
    expect(html).toContain('Codex 重置剩余 1 次');
    expect(html).toContain('2026/10/01');
    expect(html).not.toContain('role="progressbar"');
  });

  it('keeps refresh progress and cached usage in the targeted card without repeating the top-level error', () => {
    const html = renderToStaticMarkup(createElement(ProviderCard, { provider: account, now: 0, onRefresh: () => {}, refreshing: true, refreshError: '本卡片刷新失败' }));
    expect(html).toContain('aria-label="刷新 Codex 用量"');
    expect(html).toContain('disabled=""');
    expect(html).toContain('正在刷新 Codex');
    expect(html).not.toContain('本卡片刷新失败');
    expect(html).not.toContain('role="alert"');
    expect(html).toContain('待更新');
    expect(html).toContain('100% 剩余');
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

  it('preserves the last successful usage after a connection failure and leaves its error to the banner', () => {
    const html = renderToStaticMarkup(createElement(ProviderCard, {
      provider: { ...account, status: 'error', message: '网络连接失败' }, now: 0, onRefresh: () => {},
    }));
    expect(html).toContain('待更新');
    expect(html).toContain('account@example.com');
    expect(html).toContain('Plus');
    expect(html).toContain('100% 剩余');
    expect(html).toContain('role="progressbar"');
    expect(html).toContain('上次更新于');
    expect(html).not.toContain('网络连接失败');
    expect(html).not.toContain('role="alert"');
    expect(html).not.toContain('已连接');
  });

  it('does not invent usage when a failure has no validated previous result', () => {
    const html = renderToStaticMarkup(createElement(ProviderCard, {
      provider: { ...account, status: 'error', account: null, plan: '', windows: [], updatedAt: null, message: '登录已过期' }, now: 0, onRefresh: () => {},
    }));
    expect(html).toContain('待更新');
    expect(html).toContain('尚未获取用量');
    expect(html).not.toContain('role="progressbar"');
    expect(html).not.toContain('account@example.com');
    expect(html).not.toContain('role="alert"');
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
