import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { AlertCircle, LoaderCircle, RefreshCw } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  getDashboard, getSettings, saveSettings, subscribeToSettings,
  subscribeToTokenStatisticsRefresh, subscribeToUsage,
} from '../lib/api';
import {
  dismissPanel, getStatisticsPanelState, hideStatisticsPanel, presentStatisticsPanel,
  setPanelInteraction, subscribeToStatisticsPanel, type StatisticsPanelState,
} from '../lib/panel';
import { mergeDashboardSnapshot } from '../lib/snapshot';
import { useAccountStatistics } from '../lib/useAccountStatistics';
import { useContentWindowHeight } from '../lib/useContentWindowHeight';
import type { AppSettings, DashboardSnapshot, ProviderId } from '../types';
import { TokenStatisticsPanel } from './TokenStatisticsPanel';

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : typeof error === 'string' ? error : '发生未知错误，请稍后重试。';
}

export default function StatisticsWindow() {
  const [snapshot, setSnapshot] = useState<DashboardSnapshot | null>(null);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [panel, setPanel] = useState<StatisticsPanelState>({ provider: null, side: null, revision: 0 });
  const [error, setError] = useState<string | null>(null);
  const [loadAttempt, setLoadAttempt] = useState(0);
  const [saving, setSaving] = useState(false);
  const [refreshes, setRefreshes] = useState<Partial<Record<ProviderId, number>>>({});
  const root = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const lastInteraction = useRef('');
  const alive = useRef(false);
  const saveLock = useRef(false);
  const account = snapshot?.providers.find((provider) => provider.id === 'codex')?.account;
  const store = useAccountStatistics(settings, account, snapshot !== null);

  const reportHeightError = useCallback((reason: unknown) => {
    setError(`调整窗口高度失败：${errorMessage(reason)}`);
  }, []);
  const syncHeight = useContentWindowHeight({
    shell: root, content, enabled: Boolean(panel.provider), revision: panel.revision, onError: reportHeightError,
  });

  const acceptSnapshot = useCallback((incoming: DashboardSnapshot) => {
    setSnapshot((current) => mergeDashboardSnapshot(current, incoming));
  }, []);

  const acceptPanel = useCallback((incoming: StatisticsPanelState) => {
    setPanel((current) => incoming.revision >= current.revision ? incoming : current);
  }, []);

  const reportInteraction = useCallback((intent: 'pointer' | 'leave' | 'keyboard' | 'focus' = 'focus', deduplicate = false) => {
    const container = root.current;
    const focused = document.activeElement;
    const keyboard = Boolean(document.hasFocus() && container?.contains(focused) && (focused === container
      || focused?.matches(':focus-visible, select, input, textarea, [role="combobox"]')));
    const hovered = intent !== 'leave' && Boolean(container?.matches(':hover'));
    const key = `${hovered}:${keyboard}:${intent}`;
    if (deduplicate && lastInteraction.current === key) return;
    lastInteraction.current = key;
    void setPanelInteraction(hovered, keyboard, intent).catch((reason: unknown) => {
      if (alive.current) setError(`面板交互同步失败：${errorMessage(reason)}`);
    });
  }, []);

  const closeStatistics = useCallback(() => {
    void hideStatisticsPanel(true).catch((reason: unknown) => {
      if (alive.current) setError(`收起统计失败：${errorMessage(reason)}`);
    });
  }, []);

  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; };
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = settings?.theme ?? 'system';
  }, [settings?.theme]);

  useEffect(() => {
    let cancelled = false;
    let settingsEvents = 0;
    const unsubscribers: (() => void)[] = [];
    const keepSubscription = (stop: () => void) => {
      if (cancelled) stop();
      else unsubscribers.push(stop);
    };
    const failed = (label: string) => (reason: unknown) => {
      if (!cancelled) setError(`${label}：${errorMessage(reason)}`);
    };

    // Subscribe before reading: native collection or a main-window hover may
    // finish while this pre-created webview is still booting.
    void subscribeToUsage((next) => {
      if (!cancelled) acceptSnapshot(next);
    }).then(async (stop) => {
      keepSubscription(stop);
      if (cancelled) return;
      const current = await getDashboard();
      if (!cancelled) acceptSnapshot(current);
    }).catch(failed('用量同步失败'));

    void subscribeToSettings((next) => {
      settingsEvents += 1;
      if (!cancelled) setSettings(next);
    }).then(async (stop) => {
      keepSubscription(stop);
      if (cancelled) return;
      const version = settingsEvents;
      const current = await getSettings();
      if (!cancelled && version === settingsEvents) setSettings(current);
    }).catch(failed('设置同步失败'));

    void subscribeToStatisticsPanel((next) => {
      if (!cancelled) acceptPanel(next);
    }).then(async (stop) => {
      keepSubscription(stop);
      if (cancelled) return;
      const current = await getStatisticsPanelState();
      if (!cancelled) acceptPanel(current);
    }).catch(failed('统计面板连接失败'));

    void subscribeToTokenStatisticsRefresh((provider) => {
      if (!cancelled) setRefreshes((current) => ({ ...current, [provider]: (current[provider] ?? 0) + 1 }));
    }).then(keepSubscription).catch(failed('本机统计刷新连接失败'));

    return () => {
      cancelled = true;
      unsubscribers.forEach((stop) => stop());
    };
  }, [acceptPanel, acceptSnapshot, loadAttempt]);

  // The native window stays hidden until this selection has committed. Its
  // revision check discards late acknowledgements from rapid provider changes.
  useLayoutEffect(() => {
    if (!panel.provider) return;
    let cancelled = false;
    void syncHeight().then(() => { if (!cancelled) return presentStatisticsPanel(panel.revision); }).then(() => {
      if (!cancelled) reportInteraction();
    }).catch((reason: unknown) => {
      if (!cancelled) setError(`显示统计失败：${errorMessage(reason)}`);
    });
    return () => { cancelled = true; };
  }, [panel.provider, panel.revision, reportInteraction, syncHeight]);

  useEffect(() => {
    if (!panel.provider || !settings || settings.enabledProviders.includes(panel.provider)) return;
    void hideStatisticsPanel().catch((reason: unknown) => {
      if (alive.current) setError(`收起统计失败：${errorMessage(reason)}`);
    });
  }, [panel.provider, settings]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing) return;
      if (event.key === 'Escape') {
        event.preventDefault();
        void dismissPanel().catch((reason: unknown) => {
          if (alive.current) setError(`关闭面板失败：${errorMessage(reason)}`);
        });
        return;
      }
      const returnKey = panel.side === 'left' ? 'ArrowRight' : 'ArrowLeft';
      const target = event.target;
      if (event.key !== returnKey || (target instanceof HTMLElement && target.closest('input, select, textarea, [contenteditable="true"], [role="combobox"], [role="listbox"]'))) return;
      event.preventDefault();
      closeStatistics();
    };
    // Focus/blur cross native windows as well as DOM controls. Waiting for the
    // current focus event to finish makes activeElement and :focus-visible final.
    const reportAfterFocus = () => {
      queueMicrotask(() => {
        const container = root.current;
        if (container && !container.contains(document.activeElement)) container.focus({ preventScroll: true });
        reportInteraction();
      });
    };
    const reportAfterBlur = () => { queueMicrotask(() => reportInteraction()); };
    window.addEventListener('keydown', handleKeyDown);
    window.addEventListener('focus', reportAfterFocus);
    window.addEventListener('blur', reportAfterBlur);
    return () => {
      window.removeEventListener('keydown', handleKeyDown);
      window.removeEventListener('focus', reportAfterFocus);
      window.removeEventListener('blur', reportAfterBlur);
    };
  }, [closeStatistics, panel.side, reportInteraction]);

  async function savePreferences(patch: Pick<AppSettings, 'codexStatisticsSource'>) {
    if (saveLock.current) throw new Error('设置正在保存，请稍后重试。');
    saveLock.current = true;
    setSaving(true);
    try {
      // Preserve settings changed in the separate preferences window.
      const latest = await getSettings();
      const next = await saveSettings({ ...latest, ...patch });
      if (alive.current) setSettings(next);
    } finally {
      saveLock.current = false;
      if (alive.current) setSaving(false);
    }
  }

  const selected = snapshot?.providers.find((provider) => provider.id === panel.provider);
  return <div ref={root} className="app-shell app-desktop statistics-window" data-side={panel.side ?? undefined}
    role="dialog" aria-label={panel.provider ? `${panel.provider === 'codex' ? 'Codex' : 'Claude'} 使用统计` : '使用统计'} tabIndex={-1}
    onMouseEnter={() => reportInteraction('pointer')} onMouseLeave={() => reportInteraction('leave')}
    onWheelCapture={() => reportInteraction('pointer', true)} onMouseMove={() => reportInteraction('pointer', true)} onMouseDownCapture={() => reportInteraction('pointer')}
    onKeyDown={() => { queueMicrotask(() => reportInteraction('keyboard')); }}
    onFocus={() => { queueMicrotask(() => reportInteraction()); }} onBlur={() => { queueMicrotask(() => reportInteraction()); }}>
    <div ref={content} className="window-content">
    {panel.provider && <>
      {error && <div className="statistics-notice" role="alert"><AlertCircle size={15} aria-hidden="true" /><p>{error}</p><Button type="button" variant="outline" onClick={() => { setError(null); setLoadAttempt((current) => current + 1); }}><RefreshCw size={13} />重新连接</Button></div>}
      {selected && settings ? <TokenStatisticsPanel key={selected.id} provider={selected.id} name={selected.name}
        account={selected.account} settings={settings} saving={saving} accountStatisticsStore={store}
        refreshKey={`${JSON.stringify(selected)}:${refreshes[selected.id] ?? 0}`}
        onPreferencesChange={savePreferences} />
        : <aside className="token-statistics" aria-label="使用统计"><div className="statistics-header"><h2>{panel.provider === 'codex' ? 'Codex' : 'Claude'} 使用统计</h2></div><div className="statistics-state" role="status"><LoaderCircle size={22} className="spin" /><p>正在读取使用统计…</p></div></aside>}
    </>}
    </div>
  </div>;
}
