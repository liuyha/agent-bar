import type { AppSettings, ProviderId } from '../types';

export const SETTINGS_KEY = 'agentbar.settings.v1';

export function defaultSettings(): AppSettings {
  return { refreshIntervalSeconds: 300, enabledProviders: ['codex', 'claude'], theme: 'system' };
}

export function validateSettings(value: unknown): AppSettings {
  if (typeof value !== 'object' || value === null) throw new Error('设置格式不正确');
  const data = value as Record<string, unknown>;
  if (![60, 300, 900].includes(data.refreshIntervalSeconds as number)) {
    throw new Error('刷新间隔必须为 1、5 或 15 分钟');
  }
  if (!Array.isArray(data.enabledProviders)
    || !data.enabledProviders.every((id) => id === 'codex' || id === 'claude')) {
    throw new Error('包含不支持的服务');
  }
  if (data.theme !== 'system' && data.theme !== 'light' && data.theme !== 'dark') {
    throw new Error('不支持的外观设置');
  }
  return {
    refreshIntervalSeconds: data.refreshIntervalSeconds as number,
    enabledProviders: [...new Set(data.enabledProviders)] as ProviderId[],
    theme: data.theme,
  };
}
