import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { AlertCircle, Check, Clock3, Layers3, LoaderCircle, Monitor, Moon, RefreshCw, Sun, X } from 'lucide-react';
import { ProviderCard } from './components/ProviderCard';
import { TokenStatisticsPanel } from './components/TokenStatisticsPanel';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { NativeSelect } from '@/components/ui/native-select';
import { getDashboard, getSettings, hidePanel, hideSettings, isDesktop, refreshDashboard, refreshProviderDashboard, saveSettings, subscribeToSettings, subscribeToUsage, subscribeToUsageNavigation } from './lib/api';
import { mergeDashboardSnapshot } from './lib/snapshot';
import { codexStatisticsSources } from './lib/settings';
import { useAccountStatistics } from './lib/useAccountStatistics';
import { dismissPanel, getStatisticsPanelState, hideStatisticsPanel, setPanelInteraction, showStatisticsPanel, updateStatisticsPanelAnchor, subscribeToStatisticsPanel, type StatisticsPanelState } from './lib/panel';
import type { AppSettings, CodexStatisticsPreference, DashboardSnapshot, ProviderId, Theme } from './types';

const providers: { id: ProviderId; name: string; detail: string }[] = [
  { id: 'codex', name: 'Codex', detail: 'OpenAI' },
  { id: 'claude', name: 'Claude', detail: 'Anthropic' },
];

const themes: { value: Theme; name: string; icon: typeof Monitor }[] = [
  { value: 'system', name: '跟随系统', icon: Monitor },
  { value: 'light', name: '浅色', icon: Sun },
  { value: 'dark', name: '深色', icon: Moon },
];

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : typeof error === 'string' ? error : '发生未知错误，请稍后重试。';
}

function sameSettings(a: AppSettings, b: AppSettings): boolean {
  return a.refreshIntervalSeconds === b.refreshIntervalSeconds && a.theme === b.theme &&
    a.codexStatisticsSource === b.codexStatisticsSource && a.codexWebExtras === b.codexWebExtras &&
    a.enabledProviders.length === b.enabledProviders.length &&
    a.enabledProviders.every((provider) => b.enabledProviders.includes(provider));
}

function ErrorNotice({ children }: { children: ReactNode }) {
  return <div className="error-notice" role="alert"><AlertCircle size={15} aria-hidden="true" /><span>{children}</span></div>;
}

export default function App() {
  const [isSettingsWindow] = useState(() => window.location.hash === '#settings');
  const [snapshot, setSnapshot] = useState<DashboardSnapshot | null>(null);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [draft, setDraft] = useState<AppSettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshingProviders, setRefreshingProviders] = useState<Partial<Record<ProviderId, boolean>>>({});
  const [providerErrors, setProviderErrors] = useState<Partial<Record<ProviderId, string | null>>>({});
  const [statisticsRefreshes, setStatisticsRefreshes] = useState<Partial<Record<ProviderId, number>>>({});
  const [saving, setSaving] = useState(false);
  const [bootError, setBootError] = useState<string | null>(null);
  const [dashboardError, setDashboardError] = useState<string | null>(null);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [loadAttempt, setLoadAttempt] = useState(0);
  const [now, setNow] = useState(Date.now());
  const [statisticsProvider, setStatisticsProvider] = useState<ProviderId | null>(null);
  const [statisticsSide, setStatisticsSide] = useState<StatisticsPanelState['side']>(null);
  const serverStatisticsEnabled = !isDesktop && !isSettingsWindow && !loading;
  const codexAccount = snapshot?.providers.find((provider) => provider.id === 'codex')?.account;
  const accountStatisticsStore = useAccountStatistics(settings, codexAccount, serverStatisticsEnabled);
  const alive = useRef(false);
  const requestVersion = useRef(0);
  const refreshLock = useRef(false);
  const providerRefreshLocks = useRef(new Set<ProviderId>());
  const saveLock = useRef(false);
  const mainContent = useRef<HTMLElement>(null);
  const usageLayout = useRef<HTMLDivElement>(null);
  const statisticsAnchor = useRef<{ provider: ProviderId; element: HTMLElement } | null>(null);
  const lastInteraction = useRef('');
  const statisticsLeaveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  function cancelStatisticsLeave() {
    if (statisticsLeaveTimer.current !== null) clearTimeout(statisticsLeaveTimer.current);
    statisticsLeaveTimer.current = null;
  }

  function scheduleStatisticsLeave(pointer = false) {
    if (isDesktop) return;
    cancelStatisticsLeave();
    statisticsLeaveTimer.current = setTimeout(() => {
      const card = statisticsAnchor.current?.element;
      const detail = usageLayout.current?.querySelector('#token-statistics');
      const keyboardFocusInside = !pointer && (card?.contains(document.activeElement) || detail?.contains(document.activeElement)) && document.activeElement?.matches(':focus-visible');
      if (!card?.matches(':hover') && !detail?.matches(':hover') && !keyboardFocusInside) setStatisticsProvider(null);
    }, 180);
  }

  function reportInteraction(intent: 'pointer' | 'leave' | 'keyboard' | 'focus' = 'focus', deduplicate = false) {
    if (!isDesktop || isSettingsWindow) return;
    const element = statisticsAnchor.current?.element;
    const keyboard = Boolean(document.hasFocus() && element?.contains(document.activeElement) && document.activeElement?.matches(':focus-visible'));
    const hovered = intent !== 'leave' && Boolean(element?.matches(':hover'));
    const key = `${hovered}:${keyboard}:${intent}`;
    if (deduplicate && lastInteraction.current === key) return;
    lastInteraction.current = key;
    void setPanelInteraction(hovered, keyboard, intent)
      .catch((error: unknown) => { if (alive.current) setDashboardError(`面板交互同步失败：${errorMessage(error)}`); });
  }

  function openStatistics(provider: ProviderId, anchor: HTMLElement, focus = false) {
    cancelStatisticsLeave();
    statisticsAnchor.current = { provider, element: anchor };
    if (!isDesktop) { setStatisticsProvider(provider); return; }
    const bounds = measureStatisticsCard(anchor);
    if (!bounds) return;
    void showStatisticsPanel(provider, bounds, focus)
      .catch((error: unknown) => { if (alive.current) setDashboardError(`展开使用统计失败：${errorMessage(error)}`); });
    reportInteraction(focus ? 'focus' : 'pointer');
  }

  function measureStatisticsCard(anchor: HTMLElement) {
    const rect = anchor.getBoundingClientRect();
    const viewport = mainContent.current?.getBoundingClientRect();
    if (!viewport) return null;
    const x = Math.max(rect.left, viewport.left);
    const y = Math.max(rect.top, viewport.top);
    const width = Math.min(rect.right, viewport.right) - x;
    const height = Math.min(rect.bottom, viewport.bottom) - y;
    return width > 0 && height > 0 ? { x, y, width, height } : null;
  }

  function leaveStatisticsCard(insideWindow: boolean) {
    reportInteraction(insideWindow ? 'pointer' : 'leave');
    scheduleStatisticsLeave(true);
  }

  function repositionStatistics() {
    const selection = statisticsAnchor.current;
    if (!isDesktop || !statisticsProvider || !selection) return;
    const anchor = selection.element;
    const bounds = measureStatisticsCard(anchor);
    if (!bounds) {
      void hideStatisticsPanel().catch((error: unknown) => setDashboardError(errorMessage(error)));
    } else {
      void updateStatisticsPanelAnchor(selection.provider, bounds)
        .catch((error: unknown) => { if (alive.current) setDashboardError(errorMessage(error)); });
    }
  }

  useEffect(() => {
    const selection = statisticsAnchor.current;
    if (!isDesktop || !statisticsProvider || !selection || selection.provider !== statisticsProvider) return;
    const { element: anchor, provider } = selection;
    // Usage rows can change height during background refreshes. Keep the native
    // pointer hit region aligned with the visible card without reopening it.
    const observer = new ResizeObserver(() => {
      if (statisticsAnchor.current?.element !== anchor || statisticsAnchor.current.provider !== provider) return;
      const bounds = measureStatisticsCard(anchor);
      const update = bounds ? updateStatisticsPanelAnchor(provider, bounds) : hideStatisticsPanel();
      void update.catch((error: unknown) => { if (alive.current) setDashboardError(errorMessage(error)); });
    });
    observer.observe(anchor);
    if (mainContent.current) observer.observe(mainContent.current);
    return () => observer.disconnect();
  }, [statisticsProvider]);

  const acceptSnapshot = useCallback((incoming: DashboardSnapshot) => {
    setSnapshot((current) => mergeDashboardSnapshot(current, incoming));
  }, []);

  const resetUsage = useCallback(() => {
    setStatisticsProvider(null);
    setNow(Date.now());
    mainContent.current?.scrollTo({ top: 0 });
  }, []);

  const closePanel = useCallback(async () => {
    setStatisticsProvider(null);
    try {
      if (isSettingsWindow) await hideSettings();
      else await hidePanel();
    } catch (error) {
      if (!alive.current) return;
      const message = `关闭窗口失败：${errorMessage(error)}`;
      setDashboardError(message);
      setSettingsError(message);
    }
  }, [isSettingsWindow]);

  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; requestVersion.current += 1; cancelStatisticsLeave(); };
  }, []);

  useEffect(() => {
    let cancelled = false;
    requestVersion.current += 1;
    setLoading(true);
    setBootError(null);
    Promise.all([getSettings(), getDashboard()])
      .then(([nextSettings, nextSnapshot]) => {
        if (cancelled) return;
        setSettings(nextSettings);
        setDraft({ ...nextSettings, enabledProviders: [...nextSettings.enabledProviders] });
        acceptSnapshot(nextSnapshot);
      })
      .catch((error: unknown) => { if (!cancelled) setBootError(errorMessage(error)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [loadAttempt, acceptSnapshot]);

  useEffect(() => {
    document.documentElement.dataset.theme = settings?.theme ?? 'system';
  }, [settings?.theme]);

  useEffect(() => {
    if (!isDesktop || isSettingsWindow) return;
    let cancelled = false;
    let revision = -1;
    let stop: (() => void) | undefined;
    const accept = (state: StatisticsPanelState) => {
      if (cancelled || state.revision < revision) return;
      revision = state.revision;
      setStatisticsProvider(state.provider);
      setStatisticsSide(state.side);
    };
    void subscribeToStatisticsPanel(accept).then(async (unsubscribe) => {
      if (cancelled) { unsubscribe(); return; }
      stop = unsubscribe;
      accept(await getStatisticsPanelState());
    }).catch((error: unknown) => { if (!cancelled) setDashboardError(`统计窗口连接失败：${errorMessage(error)}`); });
    return () => { cancelled = true; stop?.(); };
  }, [isSettingsWindow]);

  useEffect(() => {
    const interval = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(interval);
  }, []);

  useEffect(() => {
    if (!isDesktop) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented || event.isComposing) return;
      event.preventDefault();
      if (isSettingsWindow) void closePanel();
      else void dismissPanel().catch((error: unknown) => { if (alive.current) setDashboardError(errorMessage(error)); });
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [closePanel, isSettingsWindow]);

  useEffect(() => {
    let cancelled = false;
    const unsubscribers: (() => void)[] = [];
    const keepSubscription = (unsubscribe: () => void) => {
      if (cancelled) unsubscribe();
      else unsubscribers.push(unsubscribe);
    };
    subscribeToUsage((nextSnapshot) => {
      if (cancelled) return;
      acceptSnapshot(nextSnapshot);
      setDashboardError(null);
      setNow(Date.now());
    }).then(async (unsubscribe) => {
      keepSubscription(unsubscribe);
      if (cancelled || !isDesktop) return;
      // Collection may finish between the initial cache read and listener setup.
      // Read again once subscribed; revision merging preserves any newer event.
      try {
        const currentSnapshot = await getDashboard();
        if (cancelled) return;
        acceptSnapshot(currentSnapshot);
        setNow(Date.now());
      } catch (error) {
        if (!cancelled) setDashboardError(`账号用量同步失败：${errorMessage(error)}`);
      }
    }).catch((error: unknown) => {
      if (!cancelled) setDashboardError(`自动刷新连接失败：${errorMessage(error)}`);
    });
    subscribeToSettings((nextSettings) => {
      if (cancelled) return;
      setSettings(nextSettings);
      setDraft({ ...nextSettings, enabledProviders: [...nextSettings.enabledProviders] });
    })
      .then(keepSubscription).catch((error: unknown) => {
        if (!cancelled) setDashboardError(`设置同步连接失败：${errorMessage(error)}`);
      });
    subscribeToUsageNavigation(() => { if (!cancelled && !isSettingsWindow) resetUsage(); })
      .then(keepSubscription).catch((error: unknown) => {
        if (!cancelled) setDashboardError(`托盘统计入口连接失败：${errorMessage(error)}`);
      });
    return () => {
      cancelled = true;
      unsubscribers.forEach((unsubscribe) => unsubscribe());
    };
  }, [acceptSnapshot, resetUsage, isSettingsWindow]);

  const refresh = useCallback(async () => {
    if (refreshLock.current || saveLock.current) return;
    refreshLock.current = true;
    const version = ++requestVersion.current;
    setDashboardError(null);
    try {
      const nextSnapshot = await refreshDashboard();
      if (alive.current) {
        acceptSnapshot(nextSnapshot);
        setNow(Date.now());
      }
    } catch (error) {
      if (alive.current && version === requestVersion.current) setDashboardError(`刷新失败：${errorMessage(error)}`);
    } finally {
      refreshLock.current = false;
    }
  }, [acceptSnapshot]);

  const refreshProvider = useCallback(async (provider: ProviderId) => {
    if (providerRefreshLocks.current.has(provider) || saveLock.current) return;
    providerRefreshLocks.current.add(provider);
    const version = requestVersion.current;
    setRefreshingProviders((current) => ({ ...current, [provider]: true }));
    setProviderErrors((current) => ({ ...current, [provider]: null }));
    if (provider === 'codex' && serverStatisticsEnabled && settings?.codexStatisticsSource === 'auto') void accountStatisticsStore.refresh('auto', 'manual');
    try {
      const nextSnapshot = await refreshProviderDashboard(provider);
      if (alive.current) {
        acceptSnapshot(nextSnapshot);
        setNow(Date.now());
      }
    } catch (error) {
      if (alive.current && version === requestVersion.current) setProviderErrors((current) => ({ ...current, [provider]: `刷新失败：${errorMessage(error)}` }));
    } finally {
      providerRefreshLocks.current.delete(provider);
      if (alive.current) {
        setRefreshingProviders((current) => ({ ...current, [provider]: false }));
        // Local history also refreshes when account collection returns the same unavailable state.
        setStatisticsRefreshes((current) => ({ ...current, [provider]: (current[provider] ?? 0) + 1 }));
      }
    }
  }, [acceptSnapshot, accountStatisticsStore, serverStatisticsEnabled, settings?.codexStatisticsSource]);

  useEffect(() => {
    if (isDesktop || !settings || loading || bootError) return;
    const interval = window.setInterval(() => { void refresh(); }, settings.refreshIntervalSeconds * 1_000);
    return () => window.clearInterval(interval);
  }, [settings, loading, bootError, refresh]);

  function updateDraft(patch: Partial<AppSettings>) {
    setDraft((current) => current ? { ...current, ...patch } : current);
    setSaved(false);
    setSettingsError(null);
  }

  function toggleProvider(provider: ProviderId) {
    if (!draft) return;
    updateDraft({
      enabledProviders: draft.enabledProviders.includes(provider)
        ? draft.enabledProviders.filter((id) => id !== provider)
        : providers.filter(({ id }) => id === provider || draft.enabledProviders.includes(id)).map(({ id }) => id),
    });
  }

  async function handleSave() {
    if (!draft || saveLock.current) return;
    saveLock.current = true;
    requestVersion.current += 1;
    setSaving(true);
    setSaved(false);
    setSettingsError(null);
    try {
      const nextSettings = await saveSettings(draft);
      if (!alive.current) return;
      setSettings(nextSettings);
      setDraft({ ...nextSettings, enabledProviders: [...nextSettings.enabledProviders] });
      setSaved(true);
      try {
        const nextSnapshot = await getDashboard();
        if (alive.current) {
          acceptSnapshot(nextSnapshot);
          setDashboardError(null);
          setNow(Date.now());
        }
      } catch (error) {
        if (alive.current) setDashboardError(`设置已保存，用量加载失败：${errorMessage(error)}`);
      }
    } catch (error) {
      if (alive.current) setSettingsError(`保存失败：${errorMessage(error)}`);
    } finally {
      saveLock.current = false;
      if (alive.current) setSaving(false);
    }
  }

  async function saveStatisticsPreferences(patch: Pick<AppSettings, 'codexStatisticsSource' | 'codexWebExtras'>) {
    if (!settings || saveLock.current) throw new Error('设置正在保存，请稍后重试。');
    saveLock.current = true;
    setSaving(true);
    try {
      const latest = await getSettings();
      const nextSettings = await saveSettings({ ...latest, ...patch });
      if (!alive.current) return;
      setSettings(nextSettings);
      setDraft({ ...nextSettings, enabledProviders: [...nextSettings.enabledProviders] });
    } finally {
      saveLock.current = false;
      if (alive.current) setSaving(false);
    }
  }

  const isDirty = settings && draft ? !sameSettings(settings, draft) : false;
  const visibleProviders = snapshot?.providers.filter((provider) => !settings || settings.enabledProviders.includes(provider.id)) ?? [];
  const selectedProvider = !isSettingsWindow ? visibleProviders.find((provider) => provider.id === statisticsProvider) : undefined;
  const statisticsExpanded = !isDesktop && Boolean(selectedProvider);

  return (
    <div onWheelCapture={() => reportInteraction('pointer', true)} onMouseMove={() => reportInteraction('pointer', true)} onMouseDownCapture={() => reportInteraction('pointer')} onMouseEnter={() => reportInteraction('pointer')} onMouseLeave={() => reportInteraction('leave')} onKeyDown={() => { queueMicrotask(() => reportInteraction('keyboard')); }} onFocus={() => reportInteraction()} onBlur={() => { queueMicrotask(() => reportInteraction()); }} className={`app-shell${isDesktop ? ' app-desktop' : ''}${statisticsExpanded ? ' statistics-expanded' : ''}${isSettingsWindow ? ' settings-window' : ''}`}>
      {isSettingsWindow && <header className="app-header">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true"><i /><i /><i /></span>
          <div><span className="brand-name">AgentBar</span><span className="brand-caption">AI 用量，一目了然</span></div>
        </div>
        <div className="header-actions">
          <span className="local-badge"><span />偏好设置</span>
          {isDesktop && <Button type="button" variant="ghost" size="icon" aria-label="关闭偏好设置" title="关闭偏好设置（Esc）" onClick={() => { void closePanel(); }}><X size={15} /></Button>}
        </div>
      </header>}

      <main className="main-content" ref={mainContent} onScroll={repositionStatistics}>
        {loading ? (
          <div className="state-panel" role="status"><LoaderCircle className="spin" size={25} /><h1>正在加载</h1><p>{isDesktop ? '读取本地设置与账号用量…' : '读取本地设置…'}</p></div>
        ) : bootError ? (
          <div className="state-panel"><AlertCircle size={28} /><h1>暂时无法加载</h1><p role="alert">{bootError}</p><Button type="button" onClick={() => setLoadAttempt((attempt) => attempt + 1)}><RefreshCw size={15} />重新加载</Button></div>
        ) : !isSettingsWindow ? (
          <div ref={usageLayout} className={`usage-layout${statisticsExpanded ? ' has-statistics' : ''}`} onMouseEnter={cancelStatisticsLeave} onMouseLeave={() => scheduleStatisticsLeave(true)} onFocus={cancelStatisticsLeave} onBlur={() => scheduleStatisticsLeave()}>
            <div className="usage-overview">
            {dashboardError && <ErrorNotice>{dashboardError}</ErrorNotice>}
            {visibleProviders.length > 0 ? (
              <div className="provider-list">{visibleProviders.map((provider) => <ProviderCard key={provider.id} provider={provider} now={now} active={selectedProvider?.id === provider.id} statisticsSide={statisticsSide} detachedStatistics={isDesktop} onLeaveStatistics={leaveStatisticsCard} onShowStatistics={(anchor, focus) => openStatistics(provider.id, anchor, focus)} onRefresh={() => { void refreshProvider(provider.id); }} refreshing={refreshingProviders[provider.id]} refreshDisabled={saving} refreshError={providerErrors[provider.id]} />)}</div>
            ) : (
              <div className="state-panel empty-state"><Layers3 size={29} /><h2>还没有显示的服务</h2><p>右键点击菜单栏图标，进入「偏好设置」，<br />启用 Codex 或 Claude 查看账号用量。</p></div>
            )}
            </div>
            {!isDesktop && selectedProvider && settings && <div onMouseEnter={cancelStatisticsLeave} onMouseLeave={() => scheduleStatisticsLeave(true)}><TokenStatisticsPanel key={selectedProvider.id} provider={selectedProvider.id} name={selectedProvider.name} account={selectedProvider.account} settings={settings} saving={saving} onPreferencesChange={saveStatisticsPreferences} refreshKey={`${JSON.stringify(selectedProvider)}:${statisticsRefreshes[selectedProvider.id] ?? 0}`} accountStatisticsStore={accountStatisticsStore} onClose={() => setStatisticsProvider(null)} /></div>}
          </div>
        ) : draft ? (
          <div className="settings-page">
            <div className="section-heading"><div><h1>偏好设置</h1><span>让 AgentBar 按你的习惯工作</span></div></div>
            <form onSubmit={(event) => { event.preventDefault(); void handleSave(); }}>
              <fieldset className="settings-group" disabled={saving}>
                <legend>显示的服务</legend>
                <div className="grid grid-cols-2 gap-[9px]">
                  {providers.map((provider) => <label className={`provider-option${draft.enabledProviders.includes(provider.id) ? ' selected' : ''}`} htmlFor={`provider-${provider.id}`} key={provider.id}><Checkbox id={`provider-${provider.id}`} checked={draft.enabledProviders.includes(provider.id)} disabled={saving} onCheckedChange={() => toggleProvider(provider.id)} /><span className="option-name">{provider.name}<small>{provider.detail}</small></span></label>)}
                </div>
              </fieldset>
              <fieldset className="settings-group" disabled={saving}>
                <legend>自动刷新</legend>
                <div className="setting-row"><label htmlFor="refresh-interval"><Clock3 size={15} />刷新间隔</label><NativeSelect id="refresh-interval" value={draft.refreshIntervalSeconds} onChange={(event) => updateDraft({ refreshIntervalSeconds: Number(event.target.value) })}><option value={60}>1 分钟</option><option value={300}>5 分钟</option><option value={900}>15 分钟</option></NativeSelect></div>
              </fieldset>
              <fieldset className="settings-group" disabled={saving}>
                <legend>Codex 使用统计</legend>
                <div className="setting-row"><label htmlFor="codex-statistics-source">统计来源</label><NativeSelect id="codex-statistics-source" value={draft.codexStatisticsSource} onChange={(event) => updateDraft({ codexStatisticsSource: event.target.value as CodexStatisticsPreference })}>{codexStatisticsSources.map(({ value, label }) => <option key={value} value={value}>{label}</option>)}</NativeSelect></div>
                <label className="mx-px mt-[11px] flex cursor-pointer items-center gap-1.5 text-[11px] text-secondary-foreground" htmlFor="codex-web-extras"><Checkbox id="codex-web-extras" checked={draft.codexWebExtras} disabled={saving} onCheckedChange={(checked) => updateDraft({ codexWebExtras: checked === true })} /><span>启用可选网页补充</span></label>
                <p className="statistics-footnote">本机记录提供日、周、月、年及全部统计。服务端查询账号汇总和每日 Token，由程序自动选择可用的连接方式。网页补充可单独查看 Credits 和网页用量，需连接用量网页。</p>
              </fieldset>
              <fieldset className="settings-group" disabled={saving}>
                <legend>外观</legend>
                <div className="grid grid-cols-3 gap-2">{themes.map(({ value, name, icon: Icon }) => <label className={`theme-option${draft.theme === value ? ' selected' : ''}`} key={value}><input className="sr-only" type="radio" name="theme" value={value} checked={draft.theme === value} onChange={() => updateDraft({ theme: value })} /><Icon size={17} strokeWidth={1.7} /><span>{name}</span>{draft.theme === value && <span className="theme-selected" aria-hidden="true" />}</label>)}</div>
              </fieldset>
              {settingsError && <ErrorNotice>{settingsError}</ErrorNotice>}
              <div className="save-feedback" aria-live="polite">{saved ? <><Check size={14} />设置已保存并应用</> : isDirty ? <span className="pending-feedback">有未保存的修改</span> : <span className="muted-feedback">设置仅保存在当前设备</span>}</div>
              <Button className="w-full" type="submit" disabled={saving || !isDirty}>{saving ? <LoaderCircle className="spin" size={15} /> : <Check size={15} />}{saving ? '正在保存…' : '保存设置'}</Button>
            </form>
          </div>
        ) : null}
      </main>
      <footer className="app-footer"><Monitor size={12} aria-hidden="true" /><span>{isDesktop ? '自动读取本机已登录账号' : '读取本机账号请使用桌面应用'}</span></footer>
    </div>
  );
}
