import { useCallback, useEffect, useId, useState, useSyncExternalStore } from 'react';
import { AlertCircle, ChartNoAxesCombined, LoaderCircle, RefreshCw, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { NativeSelect } from '@/components/ui/native-select';
import { getStatisticsState, refreshStatistics, subscribeToStatistics } from '../lib/tokenStatistics';
import { formatCount, formatPeriodRange, formatTime, formatTokens, formatUsd } from '../lib/format';
import { codexStatisticsSources } from '../lib/settings';
import type { createAccountStatisticsStore } from '../lib/accountStatistics';
import { periodLabels } from '../lib/statisticsPeriods';
import { AccountStatisticsView } from './AccountStatisticsContent';
import { StatisticsPeriodSwitch } from './StatisticsPeriodSwitch';
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
  if (loading && !statistics) return <div className="statistics-state" role="status"><LoaderCircle size={22} className="spin" /><p>正在统计本机会话…</p><small>首次读取历史记录可能需要一些时间</small></div>;
  if (statistics?.status !== 'ready') return <div className="statistics-state" role={error || statistics?.status === 'error' ? 'alert' : 'status'}><AlertCircle size={22} /><p>{error || statistics?.message || '暂无可统计的本机会话记录。'}</p><Button type="button" variant="outline" className="mt-1.5 text-[10px]" onClick={onRetry}><RefreshCw size={13} />重新读取</Button></div>;
  const period = statistics.periods.find((item) => item.period === selectedPeriod);

  return <>
    {error && <div className="statistics-notice" role="alert"><p>更新失败：{error} 当前显示上次统计结果。</p><Button type="button" variant="outline" onClick={onRetry}><RefreshCw size={13} />重新读取</Button></div>}
    {statistics.message && <p className="statistics-notice" role="status">{statistics.message}</p>}
    <StatisticsPeriodSwitch selectedPeriod={selectedPeriod} onPeriodChange={onPeriodChange} />
    <div className="statistics-periods">
      {period ? <section className="statistics-period" key={period.period} aria-label={`${periodLabels[period.period]} Token 统计`}>
        <div className="statistics-period-heading"><h3>{periodLabels[period.period]}</h3><span>{formatPeriodRange(period.startAt, period.endAt, period.period === 'year' || period.period === 'all')}</span></div>
        <div className="statistics-totals"><div><span>Token 用量</span><strong title={formatCount(period.totalTokens)}>{formatTokens(period.totalTokens)}</strong></div><div className="statistics-cost"><span>约等金额 · USD{period.unpricedTokens > 0 && period.estimatedCostUsd !== null ? '（部分）' : ''}</span><strong className={period.estimatedCostUsd === null ? 'cost-unknown' : undefined}>{formatUsd(period.estimatedCostUsd)}</strong></div></div>
        <dl className="statistics-counts"><div><dt>请求数</dt><dd>{period.requestCount === null ? '—' : formatCount(period.requestCount)}<small>次</small></dd></div><div><dt>会话轮次</dt><dd>{period.conversationTurns === null ? '—' : formatCount(period.conversationTurns)}<small>轮</small></dd></div></dl>
        <details className="statistics-token-details"><summary>Token 明细</summary><dl className="statistics-breakdown"><div><dt>输入（含缓存）</dt><dd title={formatCount(period.inputTokens)}>{formatTokens(period.inputTokens)}</dd></div><div><dt>输出</dt><dd title={formatCount(period.outputTokens)}>{formatTokens(period.outputTokens)}</dd></div><div><dt>缓存读取</dt><dd title={formatCount(period.cachedInputTokens)}>{formatTokens(period.cachedInputTokens)}</dd></div>{period.cacheWriteTokens > 0 && <div><dt>缓存写入</dt><dd title={formatCount(period.cacheWriteTokens)}>{formatTokens(period.cacheWriteTokens)}</dd></div>}</dl></details>
        {period.unpricedTokens > 0 && <p className="unpriced-note"><span title={formatCount(period.unpricedTokens)}>{formatTokens(period.unpricedTokens)}</span> Token 缺少可核实单价。{period.estimatedCostUsd === null ? '暂无法估算金额。' : '金额仅含已计价部分。'}</p>}
      </section> : <div className="statistics-state" role="status"><p>暂无{periodLabels[selectedPeriod]}统计数据。</p></div>}
    </div>
    <p className="statistics-footnote">本地时区 · 周一为每周起点，本年从 1 月 1 日起，全部涵盖本机保留的所有历史记录。请求数按可识别模型调用去重，会话轮次按用户发起的交互统计；缺失数据以 — 表示。仅含本机记录，可能包含多个账号。金额按公开 API 标准单价估算，不代表订阅账单或实际扣费。</p>
    <div className="statistics-updated">统计于 {formatTime(statistics.updatedAt)}</div>
  </>;
}

function LocalStatisticsView({ provider, refreshKey }: { provider: ProviderId; refreshKey: string }) {
  const subscribe = useCallback((listener: () => void) => subscribeToStatistics(provider, listener), [provider]);
  const getSnapshot = useCallback(() => getStatisticsState(provider), [provider]);
  const { statistics, loading, error } = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  const [attempt, setAttempt] = useState(0);
  const [selectedPeriod, setSelectedPeriod] = useState<TokenPeriod['period']>('day');

  useEffect(() => {
    void refreshStatistics(provider);
  }, [provider, refreshKey, attempt]);

  return <StatisticsContent statistics={statistics} loading={loading} error={error} selectedPeriod={selectedPeriod} onPeriodChange={setSelectedPeriod} onRetry={() => setAttempt((value) => value + 1)} />;
}

type StatisticsPreferences = Pick<AppSettings, 'codexStatisticsSource' | 'codexWebExtras'>;

export function TokenStatisticsPanel({ provider, name, refreshKey, accountStatisticsStore, account, settings, saving = false, onPreferencesChange, onClose }: {
  provider: ProviderId; name: string; refreshKey: string; account: string | null; settings: AppSettings;
  saving?: boolean; accountStatisticsStore: ReturnType<typeof createAccountStatisticsStore>; onPreferencesChange: (patch: StatisticsPreferences) => Promise<void>; onClose: () => void;
}) {
  const sourceId = useId();
  const webExtrasId = useId();
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const source = provider === 'codex' ? settings.codexStatisticsSource : 'local';
  async function updatePreferences(patch: Partial<StatisticsPreferences>) {
    setSettingsError(null);
    try { await onPreferencesChange({ codexStatisticsSource: settings.codexStatisticsSource, codexWebExtras: settings.codexWebExtras, ...patch }); }
    catch (reason) { setSettingsError(`保存失败：${reason instanceof Error ? reason.message : String(reason)}`); }
  }
  return <aside className={`token-statistics provider-${provider}`} id="token-statistics" aria-label={`${name} 使用统计`}>
    <div className="statistics-header"><div><ChartNoAxesCombined size={16} aria-hidden="true" /><h2>{name} 使用统计</h2></div><Button type="button" variant="ghost" size="icon" aria-label="收起 Token 统计" onClick={onClose}><X size={14} /></Button></div>
    {provider === 'codex' && <div className="mb-3 border-0 border-b border-solid border-[var(--line)] pb-2.5">
      <div className="flex min-w-0 items-center justify-between gap-2">
        <label htmlFor={sourceId} className="text-[10px] text-[var(--secondary)]">统计来源</label>
        <NativeSelect id={sourceId} value={source} disabled={saving} wrapperClassName="max-w-[70%]" className="h-auto rounded-[5px] bg-[var(--card-bg)] py-[5px] pl-1.5 text-[10px]" onChange={(event) => { void updatePreferences({ codexStatisticsSource: event.target.value as CodexStatisticsPreference }); }}>
          {codexStatisticsSources.map(({ value, label }) => <option key={value} value={value}>{label}</option>)}
        </NativeSelect>
      </div>
      <label htmlFor={webExtrasId} className={`mt-[9px] flex items-center gap-1.5 text-[10px] text-[var(--secondary)] ${saving ? 'cursor-not-allowed' : 'cursor-pointer'}`}>
        <Checkbox id={webExtrasId} checked={settings.codexWebExtras} disabled={saving} onCheckedChange={(checked) => { void updatePreferences({ codexWebExtras: checked === true }); }} />
        <span>启用网页补充</span><small className="text-[9px] text-[var(--muted)]">可选</small>
      </label>
      {settings.codexWebExtras && source === 'local' && <p className="statistics-footnote">选择服务端来源后，可连接网页查看补充数据。</p>}
      {saving && <p className="statistics-footnote" role="status">正在保存统计设置…</p>}
      {settingsError && <p className="statistics-notice" role="alert">{settingsError}</p>}
    </div>}
    {source === 'local' ? <LocalStatisticsView provider={provider} refreshKey={refreshKey} /> : <AccountStatisticsView key={`${source}:${account ?? ''}:${settings.codexWebExtras}`} source={source} webEnabled={settings.codexWebExtras} store={accountStatisticsStore} />}
  </aside>;
}
