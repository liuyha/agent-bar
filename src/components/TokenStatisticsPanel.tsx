import { useCallback, useEffect, useId, useState, useSyncExternalStore } from 'react';
import { AlertCircle, ChartNoAxesCombined, LoaderCircle, RefreshCw, X } from 'lucide-react';
import { getStatisticsState, refreshStatistics, subscribeToStatistics } from '../lib/tokenStatistics';
import { formatCount, formatPeriodRange, formatTime, formatTokens, formatUsd } from '../lib/format';
import type { ProviderId, TokenPeriod, TokenStatistics } from '../types';
import './TokenStatisticsPanel.css';

const periodLabels: Record<TokenPeriod['period'], string> = { day: '今日', week: '本周', month: '本月', year: '本年', all: '全部' };

interface StatisticsContentProps {
  statistics: TokenStatistics | null;
  loading: boolean;
  error: string | null;
  selectedPeriod?: TokenPeriod['period'];
  onPeriodChange: (period: TokenPeriod['period']) => void;
  onRetry: () => void;
}

export function StatisticsContent({ statistics, loading, error, selectedPeriod = 'day', onPeriodChange, onRetry }: StatisticsContentProps) {
  const periodGroupId = useId();
  if (loading && !statistics) return <div className="statistics-state" role="status"><LoaderCircle size={22} className="spin" /><p>正在统计本机会话…</p><small>首次读取历史记录可能需要一些时间</small></div>;
  if (statistics?.status !== 'ready') return <div className="statistics-state" role={error || statistics?.status === 'error' ? 'alert' : 'status'}><AlertCircle size={22} /><p>{error || statistics?.message || '暂无可统计的本机会话记录。'}</p><button type="button" className="secondary-button" onClick={onRetry}><RefreshCw size={13} />重新读取</button></div>;
  const period = statistics.periods.find((item) => item.period === selectedPeriod);

  return <>
    {error && <div className="statistics-notice" role="alert"><p>更新失败：{error} 当前显示上次统计结果。</p><button type="button" className="secondary-button" onClick={onRetry}><RefreshCw size={13} />重新读取</button></div>}
    {statistics.message && <p className="statistics-notice" role="status">{statistics.message}</p>}
    <div className="statistics-period-switch" role="radiogroup" aria-label="统计时段">
      {(['day', 'week', 'month', 'year', 'all'] as const).map((value) => <label key={value}>
        <input type="radio" name={periodGroupId} value={value} checked={selectedPeriod === value} onChange={() => onPeriodChange(value)} />
        <span>{periodLabels[value]}</span>
      </label>)}
    </div>
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

export function TokenStatisticsPanel({ provider, name, refreshKey, onClose }: { provider: ProviderId; name: string; refreshKey: string; onClose: () => void }) {
  const subscribe = useCallback((listener: () => void) => subscribeToStatistics(provider, listener), [provider]);
  const getSnapshot = useCallback(() => getStatisticsState(provider), [provider]);
  const { statistics, loading, error } = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  const [attempt, setAttempt] = useState(0);
  const [selectedPeriod, setSelectedPeriod] = useState<TokenPeriod['period']>('day');

  useEffect(() => {
    void refreshStatistics(provider);
  }, [provider, refreshKey, attempt]);

  return <aside className={`token-statistics provider-${provider}`} id="token-statistics" aria-label={`${name} Token 统计`}>
    <div className="statistics-header"><div><ChartNoAxesCombined size={16} aria-hidden="true" /><h2>{name} 使用统计</h2></div><button type="button" className="icon-button" aria-label="收起 Token 统计" onClick={onClose}><X size={14} /></button></div>
    <StatisticsContent statistics={statistics} loading={loading} error={error} selectedPeriod={selectedPeriod} onPeriodChange={setSelectedPeriod} onRetry={() => setAttempt((value) => value + 1)} />
  </aside>;
}
