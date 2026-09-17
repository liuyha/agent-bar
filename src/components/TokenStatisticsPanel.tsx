import { useCallback, useEffect, useId, useState, useSyncExternalStore } from 'react';
import { ChartNoAxesCombined, RefreshCw } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { NativeSelect } from '@/components/ui/native-select';
import { getStatisticsState, refreshStatistics, subscribeToStatistics } from '../lib/tokenStatistics';
import { formatCount, formatEstimatedCostCny, formatPeriodRange, formatTime, formatTokens, USD_TO_CNY_ESTIMATE_RATE } from '../lib/format';
import { codexStatisticsSources } from '../lib/settings';
import type { createAccountStatisticsStore } from '../lib/accountStatistics';
import { periodLabels } from '../lib/statisticsPeriods';
import { AccountStatisticsView } from './AccountStatisticsContent';
import { StatisticsPeriodSwitch } from './StatisticsPeriodSwitch';
import { StatisticsErrorNotice } from './StatisticsErrorNotice';
import { ActivityStatisticsSummary } from './ActivityStatisticsSummary';
import type { AppSettings, CodexStatisticsPreference, ProviderId, TokenPeriod, TokenStatistics } from '../types';
import './TokenStatisticsPanel.css';

interface StatisticsContentProps {
  statistics: TokenStatistics | null;
  loading: boolean;
  error: string | null;
  selectedPeriod?: TokenPeriod['period'];
  onPeriodChange: (period: TokenPeriod['period']) => void;
  onRetry: () => void;
}

export function StatisticsContent({ statistics, loading, error, selectedPeriod = 'day', onPeriodChange, onRetry }: StatisticsContentProps) {
  const ready = statistics?.status === 'ready';
  const failure = error || (statistics?.status === 'error' ? statistics.message || '读取本机统计失败，请重试。' : null);
  const period = ready ? statistics.periods.find((item) => item.period === selectedPeriod) : null;

  return <>
    {failure && <StatisticsErrorNotice message={`更新失败：${failure}${ready ? ' 当前显示上次统计结果。' : ''}`} busy={loading} onRetry={onRetry} />}
    {!failure && !ready && <p className="statistics-notice" role="status">{statistics?.message || (loading ? '正在统计本机会话…首次读取历史记录可能需要一些时间。' : '暂无可统计的本机会话记录。')}</p>}
    {!failure && ready && statistics.message && <p className="statistics-notice" role="status">{statistics.message}</p>}
    <ActivityStatisticsSummary statistics={statistics?.status === 'error' ? null : statistics?.activity} source="local" />
    <StatisticsPeriodSwitch selectedPeriod={selectedPeriod} onPeriodChange={onPeriodChange} />
    <div className="statistics-periods">
      {period ? <section className="statistics-period" key={period.period} aria-label={`${periodLabels[period.period]} Token 统计`}>
        <div className="statistics-period-heading"><h3>{periodLabels[period.period]}</h3><span>{formatPeriodRange(period.startAt, period.endAt, period.period === 'year' || period.period === 'all')}</span></div>
        <div className="statistics-totals"><div><span>Token 用量</span><strong title={formatCount(period.totalTokens)}>{formatTokens(period.totalTokens)}</strong></div><div className="statistics-cost"><span>约等金额 · 人民币{period.unpricedTokens > 0 && period.estimatedCostUsd !== null ? '（部分）' : ''}</span><strong className={period.estimatedCostUsd === null ? 'cost-unknown' : undefined}>{formatEstimatedCostCny(period.estimatedCostUsd)}</strong></div></div>
        <dl className="statistics-counts"><div><dt>请求数</dt><dd>{period.requestCount === null ? '—' : formatCount(period.requestCount)}<small>次</small></dd></div><div><dt>会话轮次</dt><dd>{period.conversationTurns === null ? '—' : formatCount(period.conversationTurns)}<small>轮</small></dd></div></dl>
        <details className="statistics-token-details"><summary>Token 明细</summary><dl className="statistics-breakdown"><div><dt>输入（含缓存）</dt><dd title={formatCount(period.inputTokens)}>{formatTokens(period.inputTokens)}</dd></div><div><dt>输出</dt><dd title={formatCount(period.outputTokens)}>{formatTokens(period.outputTokens)}</dd></div><div><dt>缓存读取</dt><dd title={formatCount(period.cachedInputTokens)}>{formatTokens(period.cachedInputTokens)}</dd></div>{period.cacheWriteTokens > 0 && <div><dt>缓存写入</dt><dd title={formatCount(period.cacheWriteTokens)}>{formatTokens(period.cacheWriteTokens)}</dd></div>}</dl></details>
        {period.unpricedTokens > 0 && <p className="unpriced-note"><span title={formatCount(period.unpricedTokens)}>{formatTokens(period.unpricedTokens)}</span> Token 缺少可核实单价。{period.estimatedCostUsd === null ? '暂无法估算金额。' : '金额仅含已计价部分。'}</p>}
        {period.estimatedCostUsd !== null && <p className="statistics-footnote">按固定估算汇率 1 美元 ≈ {USD_TO_CNY_ESTIMATE_RATE} 元人民币换算</p>}
      </section> : !ready || failure ? <section className="statistics-period" aria-label={`${periodLabels[selectedPeriod]} Token 统计`}>
        <div className="statistics-period-heading"><h3>{periodLabels[selectedPeriod]}</h3><span>—</span></div>
        <div className="statistics-totals"><div><span>Token 用量</span><strong>—</strong></div><div className="statistics-cost"><span>约等金额 · 人民币</span><strong className="cost-unknown">—</strong></div></div>
        <dl className="statistics-counts"><div><dt>请求数</dt><dd>—<small>次</small></dd></div><div><dt>会话轮次</dt><dd>—<small>轮</small></dd></div></dl>
        <details className="statistics-token-details"><summary>Token 明细</summary><dl className="statistics-breakdown">{['输入（含缓存）', '输出', '缓存读取'].map((label) => <div key={label}><dt>{label}</dt><dd>—</dd></div>)}</dl></details>
      </section> : <div className="statistics-state" role="status"><p>暂无{periodLabels[selectedPeriod]}统计数据。</p></div>}
    </div>
    <div className="statistics-updated">统计于 {ready ? formatTime(statistics.updatedAt) : '—'}</div>
  </>;
}

function useLocalStatisticsState(provider: ProviderId) {
  const subscribe = useCallback((listener: () => void) => subscribeToStatistics(provider, listener), [provider]);
  const getSnapshot = useCallback(() => getStatisticsState(provider), [provider]);
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

function LocalStatisticsView({ provider, refreshKey }: { provider: ProviderId; refreshKey: string }) {
  const { statistics, loading, error } = useLocalStatisticsState(provider);
  const [attempt, setAttempt] = useState(0);
  const [selectedPeriod, setSelectedPeriod] = useState<TokenPeriod['period']>('day');

  useEffect(() => {
    void refreshStatistics(provider);
  }, [provider, refreshKey, attempt]);

  return <StatisticsContent statistics={statistics} loading={loading} error={error} selectedPeriod={selectedPeriod} onPeriodChange={setSelectedPeriod} onRetry={() => setAttempt((value) => value + 1)} />;
}

type StatisticsPreferences = Pick<AppSettings, 'codexStatisticsSource'>;

export function TokenStatisticsPanel({ provider, name, refreshKey, accountStatisticsStore, account, settings, saving = false, onPreferencesChange }: {
  provider: ProviderId; name: string; refreshKey: string; account: string | null; settings: AppSettings;
  saving?: boolean; accountStatisticsStore: ReturnType<typeof createAccountStatisticsStore>; onPreferencesChange: (patch: StatisticsPreferences) => Promise<void>;
}) {
  const sourceId = useId();
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const source = provider === 'codex' ? settings.codexStatisticsSource : 'local';
  const localState = useLocalStatisticsState(provider);
  const serverState = useSyncExternalStore(accountStatisticsStore.subscribe, accountStatisticsStore.getSnapshot, accountStatisticsStore.getSnapshot);
  const loading = source === 'local' ? localState.loading : serverState.loading;
  const refreshing = source === 'local' ? localState.loading : serverState.refreshing;
  const refreshLabel = source === 'local' ? '刷新本机统计' : '刷新服务端统计';
  async function updatePreferences(patch: Partial<StatisticsPreferences>) {
    setSettingsError(null);
    try { await onPreferencesChange({ codexStatisticsSource: settings.codexStatisticsSource, ...patch }); }
    catch (reason) { setSettingsError(`保存失败：${reason instanceof Error ? reason.message : String(reason)}`); }
  }
  return <aside className={`token-statistics provider-${provider}`} id="token-statistics" aria-label={`${name} 使用统计`}>
    <div className="statistics-header">
      <div className="statistics-title"><ChartNoAxesCombined size={16} aria-hidden="true" /><h2>{name} 使用统计</h2></div>
      {provider === 'codex' && <div className="statistics-source-control">
        <label htmlFor={sourceId}>统计来源</label>
        <NativeSelect id={sourceId} value={source} disabled={saving} wrapperClassName="w-[76px] shrink-0" className="h-auto rounded-[5px] bg-[var(--card-bg)] py-[5px] pl-1.5 text-[10px]" onChange={(event) => { void updatePreferences({ codexStatisticsSource: event.target.value as CodexStatisticsPreference }); }}>
          {codexStatisticsSources.map(({ value, label }) => <option key={value} value={value}>{label}</option>)}
        </NativeSelect>
      </div>}
      <Button variant="outline" size="icon" type="button" aria-label={refreshLabel} aria-busy={refreshing} title={refreshing ? '正在刷新统计…' : refreshLabel} disabled={loading || refreshing} onClick={() => { void (source === 'local' ? refreshStatistics(provider) : accountStatisticsStore.refresh(source, 'manual')); }}><RefreshCw size={13} className={refreshing ? 'spin' : undefined} /></Button>
    </div>
    {provider === 'codex' && <>
      {saving && <p className="statistics-footnote" role="status">正在保存统计设置…</p>}
      {settingsError && <p className="statistics-notice" role="alert">{settingsError}</p>}
    </>}
    {source === 'local' ? <LocalStatisticsView provider={provider} refreshKey={refreshKey} /> : <AccountStatisticsView key={`${source}:${account ?? ''}`} source={source} store={accountStatisticsStore} />}
  </aside>;
}
