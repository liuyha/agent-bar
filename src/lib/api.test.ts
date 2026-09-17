import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DashboardSnapshot } from '../types';
import { defaultSettings, SETTINGS_KEY } from './settings';

const native = vi.hoisted(() => ({ desktop: false, invoke: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => native.desktop, invoke: native.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: native.listen }));

beforeEach(() => {
  vi.resetModules();
  vi.clearAllMocks();
  native.desktop = false;
  const values = new Map<string, string>();
  vi.stubGlobal('window', {
    localStorage: {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    },
  });
});

afterEach(() => vi.unstubAllGlobals());

describe('browser dashboard', () => {
  it('does not install native account refresh listeners in browser previews', async () => {
    const { subscribeToAccountStatisticsRefresh } = await import('./api');
    const refresh = vi.fn();
    const stop = await subscribeToAccountStatisticsRefresh(refresh);
    stop();
    expect(native.listen).not.toHaveBeenCalled();
    expect(refresh).not.toHaveBeenCalled();
  });

  it('does not fabricate remote statistics or open an authenticated webpage in browser preview', async () => {
    const { getCachedCodexAccountStatistics, getCodexAccountStatistics, openCodexUsageWeb } = await import('./api');
    expect(await getCachedCodexAccountStatistics('oauth')).toBeNull();
    expect(await getCodexAccountStatistics('oauth')).toMatchObject({
      source: 'oauth', status: 'unavailable', account: null, accountId: null,
      summary: { lifetimeTokens: null, peakDailyTokens: null }, dailyUsage: null, updatedAt: null, web: null,
    });
    await expect(openCodexUsageWeb()).rejects.toThrow('桌面应用');
    expect(native.invoke).not.toHaveBeenCalled();
  });
  it('shows unavailable providers without inventing account or usage data', async () => {
    const { getDashboard, refreshDashboard } = await import('./api');
    const initial = await getDashboard();
    const next = await refreshDashboard();
    expect(initial.mode).toBe('live');
    expect(initial.providers.map((provider) => provider.id)).toEqual(['codex', 'claude']);
    for (const provider of initial.providers) {
      expect(provider).toMatchObject({
        source: 'local', status: 'unavailable', account: null, plan: '', windows: [], updatedAt: null,
      });
      expect(provider.message).toContain('桌面应用');
    }
    expect(next.revision).toBeGreaterThan(initial.revision);
    expect(native.invoke).not.toHaveBeenCalled();
  });

  it('preserves browser settings and includes only enabled providers', async () => {
    const { getDashboard, getSettings, saveSettings } = await import('./api');
    const settings = { ...defaultSettings(), enabledProviders: ['claude' as const], theme: 'dark' as const };
    await saveSettings(settings);
    expect(await getSettings()).toEqual(settings);
    expect((await getDashboard()).providers.map((provider) => provider.id)).toEqual(['claude']);
    await saveSettings({ ...settings, enabledProviders: [] });
    expect((await getDashboard()).providers).toEqual([]);
    expect(native.invoke).not.toHaveBeenCalled();
  });

  it('recovers from damaged stored settings without creating account data', async () => {
    window.localStorage.setItem(SETTINGS_KEY, '{broken');
    const { getDashboard } = await import('./api');
    const snapshot = await getDashboard();
    expect(snapshot.providers).toHaveLength(2);
    expect(snapshot.providers.every((provider) => provider.windows.length === 0)).toBe(true);
  });

  it('does not invent browser data during a card refresh and rejects disabled providers', async () => {
    const { refreshProviderDashboard, saveSettings } = await import('./api');
    const snapshot = await refreshProviderDashboard('codex');
    expect(snapshot.providers.every((provider) => provider.status === 'unavailable')).toBe(true);
    expect(snapshot.providers.every((provider) => provider.updatedAt === null)).toBe(true);
    await saveSettings({ ...defaultSettings(), enabledProviders: ['codex'] });
    await expect(refreshProviderDashboard('claude')).rejects.toThrow('当前服务未启用');
    expect(native.invoke).not.toHaveBeenCalled();
  });
});

describe('desktop dashboard', () => {
  const snapshot: DashboardSnapshot = { revision: 2, mode: 'live', providers: [], updatedAt: '2026-09-16T00:00:00Z' };

  it('delivers explicit account statistics refreshes and releases the native listener', async () => {
    native.desktop = true;
    const unsubscribe = vi.fn();
    native.listen.mockResolvedValue(unsubscribe);
    const { subscribeToAccountStatisticsRefresh } = await import('./api');
    const refresh = vi.fn();
    const stop = await subscribeToAccountStatisticsRefresh(refresh);
    expect(native.listen).toHaveBeenCalledWith('refresh-account-statistics', refresh);
    native.listen.mock.calls[0][1]();
    expect(refresh).toHaveBeenCalledOnce();
    stop();
    expect(unsubscribe).toHaveBeenCalledOnce();
  });

  it('reads only the selected account statistics cache and propagates cache failures', async () => {
    native.desktop = true;
    const cached = { source: 'oauth', status: 'ready' };
    native.invoke.mockResolvedValueOnce(cached).mockResolvedValueOnce(null).mockRejectedValueOnce(new Error('缓存损坏'));
    const { getCachedCodexAccountStatistics } = await import('./api');
    expect(await getCachedCodexAccountStatistics('oauth')).toEqual(cached);
    expect(await getCachedCodexAccountStatistics('pat')).toBeNull();
    await expect(getCachedCodexAccountStatistics('auto')).rejects.toThrow('缓存损坏');
    expect(native.invoke.mock.calls).toEqual([
      ['get_cached_codex_account_statistics', { source: 'oauth' }],
      ['get_cached_codex_account_statistics', { source: 'pat' }],
      ['get_cached_codex_account_statistics', { source: 'auto' }],
    ]);
  });

  it('queries the selected server source and exposes webpage disabled errors', async () => {
    native.desktop = true;
    native.invoke.mockResolvedValueOnce({ source: 'pat', status: 'ready' }).mockRejectedValueOnce(new Error('网页补充尚未启用'));
    const { getCodexAccountStatistics, openCodexUsageWeb } = await import('./api');
    expect(await getCodexAccountStatistics('pat')).toEqual({ source: 'pat', status: 'ready' });
    await expect(openCodexUsageWeb()).rejects.toThrow('尚未启用');
    expect(native.invoke.mock.calls).toEqual([
      ['get_codex_account_statistics', { source: 'pat' }], ['open_codex_usage_web'],
    ]);
  });

  it('delivers saved preferences to other windows and releases its listener', async () => {
    native.desktop = true;
    const unsubscribe = vi.fn();
    native.listen.mockResolvedValue(unsubscribe);
    const { subscribeToSettings } = await import('./api');
    const accept = vi.fn();
    const stop = await subscribeToSettings(accept);
    expect(native.listen.mock.calls[0][0]).toBe('settings-updated');
    const settings = { ...defaultSettings(), theme: 'dark' as const, enabledProviders: ['claude' as const] };
    native.listen.mock.calls[0][1]({ payload: settings });
    expect(accept).toHaveBeenCalledWith(settings);
    stop();
    expect(unsubscribe).toHaveBeenCalledOnce();
  });

  it('closes preferences independently of the usage panel', async () => {
    native.desktop = true;
    native.invoke.mockResolvedValue(undefined);
    const { hideSettings } = await import('./api');
    await hideSettings();
    expect(native.invoke.mock.calls).toEqual([['hide_settings']]);
  });

  it('returns native snapshots for initial loads and explicit refreshes', async () => {
    native.desktop = true;
    native.invoke.mockResolvedValue(snapshot);
    const { getDashboard, refreshDashboard } = await import('./api');
    expect(await getDashboard()).toBe(snapshot);
    expect(await refreshDashboard()).toBe(snapshot);
    expect(native.invoke.mock.calls).toEqual([['get_dashboard'], ['refresh_dashboard']]);
  });

  it('surfaces native failures without replacing them with generated data', async () => {
    native.desktop = true;
    native.invoke.mockRejectedValue(new Error('账号读取失败'));
    const { getDashboard, refreshDashboard } = await import('./api');
    await expect(getDashboard()).rejects.toThrow('账号读取失败');
    await expect(refreshDashboard()).rejects.toThrow('账号读取失败');
  });

  it('refreshes only the provider named by the card without invoking a dashboard refresh', async () => {
    native.desktop = true;
    native.invoke.mockResolvedValue(snapshot);
    const { refreshProviderDashboard } = await import('./api');
    expect(await refreshProviderDashboard('claude')).toBe(snapshot);
    expect(native.invoke.mock.calls).toEqual([['refresh_provider_dashboard', { provider: 'claude' }]]);
    native.invoke.mockRejectedValue(new Error('当前服务刷新失败'));
    await expect(refreshProviderDashboard('codex')).rejects.toThrow('当前服务刷新失败');
  });

  it('requests statistics for the selected provider and serializes panel resizes', async () => {
    native.desktop = true;
    native.invoke.mockResolvedValue({ status: 'ready', periods: [] });
    const { getTokenStatistics, setPanelExpanded } = await import('./api');
    expect(await getTokenStatistics('claude')).toEqual({ status: 'ready', periods: [] });
    await Promise.all([setPanelExpanded(true), setPanelExpanded(false)]);
    expect(native.invoke.mock.calls).toEqual([
      ['get_token_statistics', { provider: 'claude' }],
      ['set_panel_expanded', { expanded: true }],
      ['set_panel_expanded', { expanded: false }],
    ]);
  });

  it('reads the saved statistics without invoking collection', async () => {
    native.desktop = true;
    const cached = { status: 'ready', periods: [], updatedAt: '2026-09-17T00:00:00Z' };
    native.invoke.mockResolvedValueOnce(cached).mockResolvedValueOnce(null);
    const { getCachedTokenStatistics } = await import('./api');
    expect(await getCachedTokenStatistics('codex')).toEqual(cached);
    expect(await getCachedTokenStatistics('claude')).toBeNull();
    expect(native.invoke.mock.calls).toEqual([
      ['get_cached_token_statistics', { provider: 'codex' }],
      ['get_cached_token_statistics', { provider: 'claude' }],
    ]);
  });
});
