import { useEffect, useLayoutEffect, useState } from 'react';
import type { AppSettings } from '../types';
import { createAccountStatisticsStore } from './accountStatistics';
import { subscribeToAccountStatisticsRefresh } from './api';

/** Keep one remote statistics owner alive even while its window is hidden. */
export function useAccountStatistics(
  settings: AppSettings | null,
  account: string | null | undefined,
  enabled: boolean,
): ReturnType<typeof createAccountStatisticsStore> {
  const [store] = useState(createAccountStatisticsStore);
  const remoteEnabled = enabled && settings?.codexStatisticsSource === 'auto'
    && settings.enabledProviders.includes('codex');
  const intervalSeconds = settings?.refreshIntervalSeconds;

  // Invalidate in-flight work and clear the prior account before the next paint.
  // The native cache reader independently validates authentication and settings.
  useLayoutEffect(() => {
    if (!remoteEnabled) return;
    void store.initialize('auto');
    return store.cancel;
  }, [store, remoteEnabled, account, settings?.codexWebExtras]);

  useEffect(() => {
    if (!remoteEnabled || !intervalSeconds) return;
    const interval = window.setInterval(() => { void store.refresh('auto'); }, intervalSeconds * 1_000);
    return () => window.clearInterval(interval);
  }, [store, remoteEnabled, intervalSeconds]);

  useEffect(() => {
    if (!remoteEnabled) return;
    let cancelled = false;
    let unsubscribe: (() => void) | undefined;
    void subscribeToAccountStatisticsRefresh(() => {
      if (!cancelled) void store.refresh('auto', 'manual');
    }).then((stop) => {
      if (cancelled) stop();
      else unsubscribe = stop;
    }).catch((error: unknown) => {
      if (!cancelled) console.error('统计刷新入口连接失败：', error);
    });
    return () => { cancelled = true; unsubscribe?.(); };
  }, [store, remoteEnabled]);

  return store;
}
