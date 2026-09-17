import { useState, useSyncExternalStore } from 'react';
import { createAccountStatisticsStore, formatDuration, type AccountStatisticsState } from '../lib/accountStatistics';
import { accountStatisticsPeriod } from '../lib/accountStatisticsPeriods';
import { formatCount, formatTokens } from '../lib/format';
import { periodLabels } from '../lib/statisticsPeriods';
import { StatisticsPeriodSwitch } from './StatisticsPeriodSwitch';
import { StatisticsErrorNotice } from './StatisticsErrorNotice';
import type { AccountUsageSnapshot, CodexStatisticsSource, TokenPeriod } from '../types';

function fullTime(value: string | null): string {
  if (!value || !Number.isFinite(Date.parse(value))) return '—';
  if (/^\d{4}-\d{2}-\d{2}$/.test(value)) return value;
  return new Date(value).toLocaleString('zh-CN', { hour12: false });
}

function number(value: number | null, suffix = ''): string {
  return value === null || !Number.isFinite(value) ? '—' : `${value.toLocaleString('zh-CN', { maximumFractionDigits: 4 })}${suffix}`;
}

interface Props extends AccountStatisticsState {
  selectedPeriod?: TokenPeriod['period'];
  onPeriodChange: (period: TokenPeriod['period']) => void;
  onRetry: () => void;
}

function AccountPeriodStatistics({ statistics, selectedPeriod }: { statistics: AccountUsageSnapshot | null; selectedPeriod: TokenPeriod['period'] }) {
  const period = statistics ? accountStatisticsPeriod(statistics, selectedPeriod) : null;
  const daily = period?.dailyUsage ?? null;
  const totalTokens = period?.totalTokens ?? null;
  const peakDailyTokens = period?.peakDailyTokens ?? null;
  const maxDaily = daily?.reduce((maximum, day) => Math.max(maximum, day.tokens), 0) ?? 0;
  const all = selectedPeriod === 'all';
  return <section className="statistics-period account-period-statistics" aria-label={`${periodLabels[selectedPeriod]}服务端 Token 统计`}>
    <div className="statistics-period-heading"><h3>{periodLabels[selectedPeriod]}</h3><span>{all ? '覆盖范围以服务端为准' : !period ? '—' : period.startDate === period.endDate ? period.endDate : `${period.startDate} – ${period.endDate}`}</span></div>
    <dl className="account-statistics-metrics account-period-metrics">
      <div><dt>{all ? '累计 Token' : 'Token 合计'}</dt><dd title={totalTokens === null ? undefined : formatCount(totalTokens)}>{totalTokens === null ? '—' : formatTokens(totalTokens)}</dd></div>
      <div><dt>{all ? '单日峰值 Token' : '单日峰值'}</dt><dd title={peakDailyTokens === null ? undefined : formatCount(peakDailyTokens)}>{peakDailyTokens === null ? '—' : formatTokens(peakDailyTokens)}</dd></div>
    </dl>
    <section className="account-daily-statistics" aria-label="服务端每日 Token 用量记录"><h3>每日 Token 用量记录</h3>
      {!statistics ? <p className="statistics-footnote">—</p> : daily === null ? <p className="statistics-footnote">服务端未提供每日 Token 用量记录。</p> : daily.length === 0 ? <p className="statistics-footnote">{statistics.dailyUsage?.length === 0 ? '服务端返回的每日记录为空。' : `服务端尚未返回${periodLabels[selectedPeriod]}日期范围内的每日记录。`}</p> : <><p className="statistics-footnote">已返回 {daily.length} 条日期记录 · {daily[0].date} 至 {daily[daily.length - 1].date}</p><ul className="account-daily-list">{daily.map((day) => <li key={day.date}><time>{day.date}</time><span className="account-daily-bar" aria-hidden="true"><i style={{ width: `${maxDaily > 0 ? Math.max(0, day.tokens / maxDaily * 100) : 0}%` }} /></span><span title={formatCount(day.tokens)}>{formatTokens(day.tokens)}</span></li>)}</ul></>}
    </section>
  </section>;
}

export function AccountStatisticsContent({ statistics, loading, refreshing, error, selectedPeriod = 'day', onPeriodChange, onRetry }: Props) {
  const ready = statistics?.status === 'ready';
  const summary = ready ? statistics.summary : null;
  const busy = loading || refreshing;
  const failure = error || (statistics?.status === 'error' ? statistics.message || '读取服务端统计失败，请重试。' : null);
  const statusMessage = ready
    ? loading ? '正在更新服务端统计…' : statistics.message
    : busy ? loading ? '正在读取服务端统计…' : '暂无本地缓存，正在后台获取服务端统计…' : statistics?.message || '暂无服务端统计缓存。';
  return <>
    {failure ? <StatisticsErrorNotice message={`更新失败：${failure}${ready ? ' 当前显示本地缓存。' : ''}`} busy={busy} onRetry={onRetry} /> : statusMessage && <p className="statistics-notice" role="status">{statusMessage}</p>}
    <StatisticsPeriodSwitch selectedPeriod={selectedPeriod} onPeriodChange={onPeriodChange} />
    <AccountPeriodStatistics statistics={ready ? statistics : null} selectedPeriod={selectedPeriod} />
    <details className="account-statistics-details"><summary>账号活动概览</summary><dl className="account-statistics-metrics account-activity-metrics">
      <div><dt>最长任务时长</dt><dd>{formatDuration(summary?.longestRunningTurnSec ?? null)}</dd></div>
      <div><dt>当前连续活跃</dt><dd>{number(summary?.currentStreakDays ?? null, ' 天')}</dd></div>
      <div><dt>最长连续活跃</dt><dd>{number(summary?.longestStreakDays ?? null, ' 天')}</dd></div>
    </dl></details>
    <div className="statistics-updated">服务端更新于 {fullTime(ready ? statistics.serviceUpdatedAt : null)}</div>
    <div className="statistics-updated">采集于 {fullTime(ready ? statistics.updatedAt : null)}</div>
  </>;
}

export function AccountStatisticsView({ source, store }: { source: Exclude<CodexStatisticsSource, 'local'>; store: ReturnType<typeof createAccountStatisticsStore> }) {
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  const [selectedPeriod, setSelectedPeriod] = useState<TokenPeriod['period']>('day');
  return <AccountStatisticsContent {...state} selectedPeriod={selectedPeriod} onPeriodChange={setSelectedPeriod} onRetry={() => { void store.refresh(source, 'manual'); }} />;
}
