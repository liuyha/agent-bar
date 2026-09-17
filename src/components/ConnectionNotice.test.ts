import { createElement, Fragment } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { ProviderUsage } from '../types';
import { ConnectionNotice } from './ConnectionNotice';
import { ProviderCard } from './ProviderCard';

const codex: ProviderUsage = {
  id: 'codex', name: 'Codex', source: 'local', status: 'error', plan: 'Plus',
  account: 'account@example.com', message: '网络连接超时', updatedAt: '2026-09-17T00:00:00Z',
  windows: [{ label: '5 小时用量', usedPercent: 38, resetsAt: null }],
};

describe('connection failure banner', () => {
  it('shows one top-level alert while keeping the account statistics below it', () => {
    const html = renderToStaticMarkup(createElement(Fragment, null,
      createElement(ConnectionNotice, { providers: [codex], errors: {} }),
      createElement(ProviderCard, { provider: codex, now: 0 }),
    ));
    expect(html.match(/role="alert"/g)).toHaveLength(1);
    expect(html.indexOf('网络连接超时')).toBeLessThan(html.indexOf('<article'));
    expect(html).toContain('连接失败');
    expect(html).toContain('Codex：网络连接超时');
    expect(html).toContain('保留上次成功获取的用量');
    expect(html).toContain('62% 剩余');
  });

  it('groups failures for enabled providers without duplicating provider and request errors', () => {
    const claude: ProviderUsage = { ...codex, id: 'claude', name: 'Claude', status: 'ready', message: null };
    const html = renderToStaticMarkup(createElement(ConnectionNotice, {
      providers: [codex, claude], errors: { codex: '刷新请求失败', claude: '连接中断' },
    }));
    expect(html.match(/role="alert"/g)).toHaveLength(1);
    expect(html).toContain('Codex：刷新请求失败');
    expect(html).toContain('Claude：连接中断');
    expect(html).not.toContain('网络连接超时');
  });

  it('does not claim to show previous data when no usage was ever retrieved', () => {
    const html = renderToStaticMarkup(createElement(ConnectionNotice, {
      providers: [{ ...codex, windows: [], updatedAt: null, message: null }], errors: {},
    }));
    expect(html).toContain('暂时无法读取账号用量');
    expect(html).not.toContain('保留上次成功获取');
  });

  it('disappears after recovery and ignores errors belonging to hidden providers', () => {
    const html = renderToStaticMarkup(createElement(ConnectionNotice, {
      providers: [{ ...codex, status: 'ready', message: null }], errors: { claude: '连接失败' },
    }));
    expect(html).toBe('');
  });
});
