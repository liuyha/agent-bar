import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { AppSettings, DashboardSnapshot, ProviderId, TokenStatistics } from '../types';
import { defaultSettings, SETTINGS_KEY, validateSettings } from './settings';

export const isDesktop = isTauri();
let browserRevision = 0;

async function browserDashboard(): Promise<DashboardSnapshot> {
  const settings = await getSettings();
  return {
    revision: ++browserRevision,
    providers: settings.enabledProviders.map((id) => ({
      id,
      name: id === 'codex' ? 'Codex' : 'Claude',
      plan: '',
      source: 'local',
      status: 'unavailable',
      account: null,
      message: '请通过桌面应用运行 AgentBar，读取本机已登录的账号。',
      windows: [],
      updatedAt: null,
    })),
    updatedAt: new Date().toISOString(),
    mode: 'live',
  };
}

export async function getSettings(): Promise<AppSettings> {
  if (isDesktop) return invoke<AppSettings>('get_settings');
  const raw = window.localStorage.getItem(SETTINGS_KEY);
  if (!raw) return defaultSettings();
  try {
    return validateSettings(JSON.parse(raw));
  } catch {
    // An older or damaged setting must not prevent opening Settings.
    return defaultSettings();
  }
}

export async function saveSettings(settings: AppSettings): Promise<AppSettings> {
  const validated = validateSettings(settings);
  if (isDesktop) return invoke<AppSettings>('save_settings', { settings: validated });
  window.localStorage.setItem(SETTINGS_KEY, JSON.stringify(validated));
  return validated;
}

export async function getDashboard(): Promise<DashboardSnapshot> {
  if (isDesktop) return invoke<DashboardSnapshot>('get_dashboard');
  return browserDashboard();
}

export async function refreshDashboard(): Promise<DashboardSnapshot> {
  if (isDesktop) return invoke<DashboardSnapshot>('refresh_dashboard');
  return browserDashboard();
}

export async function refreshProviderDashboard(provider: ProviderId): Promise<DashboardSnapshot> {
  if (isDesktop) return invoke<DashboardSnapshot>('refresh_provider_dashboard', { provider });
  const settings = await getSettings();
  if (!settings.enabledProviders.includes(provider)) throw new Error('当前服务未启用');
  // Browser previews have no account access; no provider is actually collected here.
  return browserDashboard();
}

export async function subscribeToUsage(callback: (snapshot: DashboardSnapshot) => void): Promise<() => void> {
  if (!isDesktop) return () => {};
  return listen<DashboardSnapshot>('usage-updated', ({ payload }) => callback(payload));
}

export async function subscribeToSettings(callback: (settings: AppSettings) => void): Promise<() => void> {
  if (!isDesktop) return () => {};
  return listen<AppSettings>('settings-updated', ({ payload }) => callback(payload));
}

export async function subscribeToUsageNavigation(callback: () => void): Promise<() => void> {
  if (!isDesktop) return () => {};
  return listen('navigate-usage', callback);
}

export async function hidePanel(): Promise<void> {
  if (isDesktop) await invoke('hide_panel');
}

export async function hideSettings(): Promise<void> {
  if (isDesktop) await invoke('hide_settings');
}

export async function getTokenStatistics(provider: ProviderId): Promise<TokenStatistics> {
  if (isDesktop) return invoke<TokenStatistics>('get_token_statistics', { provider });
  return {
    status: 'unavailable',
    message: '请通过桌面应用读取本机会话记录，查看 Token 用量与约等金额。',
    periods: [],
    updatedAt: new Date().toISOString(),
  };
}

export async function getCachedTokenStatistics(provider: ProviderId): Promise<TokenStatistics | null> {
  if (isDesktop) return invoke<TokenStatistics | null>('get_cached_token_statistics', { provider });
  return null;
}

// Keep rapid hover / keyboard navigation from applying native sizes out of order.
let panelResize = Promise.resolve();
export function setPanelExpanded(expanded: boolean): Promise<void> {
  if (!isDesktop) return Promise.resolve();
  panelResize = panelResize.catch(() => {}).then(() => invoke<void>('set_panel_expanded', { expanded }));
  return panelResize;
}
