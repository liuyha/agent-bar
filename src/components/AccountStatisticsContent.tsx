import { useState, useSyncExternalStore } from 'react';
import { createAccountStatisticsStore, type AccountStatisticsState } from '../lib/accountStatistics';
import { accountStatisticsPeriod } from '../lib/accountStatisticsPeriods';
import { formatCount, formatTokens } from '../lib/format';
import { periodLabels } from '../lib/statisticsPeriods';
import { dateRangeError, dateRangeLabel, emptyDateRange, hasDateRange, type StatisticsDateRange } from '../lib/statisticsDateRange';
import { StatisticsDateRangeFilter } from './StatisticsDateRangeFilter';
import { StatisticsPeriodSwitch } from './StatisticsPeriodSwitch';
import { StatisticsErrorNotice } from './StatisticsErrorNotice';
import { ActivityStatisticsSummary } from './ActivityStatisticsSummary';
import type { AccountUsageSnapshot, CodexStatisticsSource, TokenPeriod } from '../types';

function fullTime(value: string | null): string {
  if (!value || !Number.isFinite(Date.parse(value))) return '—';
  if (/^\d{4}-\d{2}-\d{2}$/.test(value)) return value;
  return new Date(value).toLocaleString('zh-CN', { hour12: false });
}

interface Props extends AccountStatisticsState {
  selectedPeriod?: TokenPeriod['period'];
  onPeriodChange: (period: TokenPeriod['period']) => void;
  onRetry: () => void;
  dateRange?: StatisticsDateRange;
  onDateRangeChange?: (range: StatisticsDateRange) => void;
}

function AccountPeriodStatistics({ statistics, selectedPeriod, dateRange }: { statistics: AccountUsageSnapshot | null; selectedPeriod: TokenPeriod['period']; dateRange: StatisticsDateRange }) {
  const filtered = selectedPeriod === 'all' && hasDateRange(dateRange);
  const invalid = filtered && Boolean(dateRangeError(dateRange));
  const period = statistics ? accountStatisticsPeriod(statistics, selectedPeriod, new Date(), dateRange) : null;
  const daily = period?.dailyUsage ?? null;
  const totalTokens = period?.totalTokens ?? null;
  const peakDailyTokens = period?.peakDailyTokens ?? null;
  const maxDaily = daily?.reduce((maximum, day) => Math.max(maximum, day.tokens), 0) ?? 0;
  const all = selectedPeriod === 'all' && !filtered;
  return <section className="statistics-period account-period-statistics" aria-label={`${periodLabels[selectedPeriod]}服务端 Token 统计`}>
    <div className="statistics-period-heading"><h3>{periodLabels[selectedPeriod]}</h3><span>{filtered ? dateRangeLabel(dateRange) : all ? '覆盖范围以服务端为准' : !period ? '—' : period.startDate === period.endDate ? period.endDate : `${period.startDate} – ${period.endDate}`}</span></div>
    {filtered && !invalid && <p className="statistics-footnote">仅汇总所选日期内服务端已返回的记录。</p>}
    <dl className="account-statistics-metrics account-period-metrics">
      <div><dt>{all ? '累计 Token' : 'Token 合计'}</dt><dd title={totalTokens === null ? undefined : formatCount(totalTokens)}>{totalTokens === null ? '—' : formatTokens(totalTokens)}</dd></div>
      <div><dt>{all ? '单日峰值 Token' : '单日峰值'}</dt><dd title={peakDailyTokens === null ? undefined : formatCount(peakDailyTokens)}>{peakDailyTokens === null ? '—' : formatTokens(peakDailyTokens)}</dd></div>
    </dl>
    <section className="account-daily-statistics" aria-label="服务端每日 Token 用量记录"><h3>每日 Token 用量记录</h3>
      {invalid ? <p className="statistics-footnote">请调整日期范围后查看统计。</p> : !statistics ? <p className="statistics-footnote">—</p> : daily === null ? <p className="statistics-footnote">服务端未提供每日 Token 用量记录。</p> : daily.length === 0 ? <p className="statistics-footnote">{statistics.dailyUsage?.length === 0 ? '服务端返回的每日记录为空。' : `服务端尚未返回${filtered ? '所选' : periodLabels[selectedPeriod]}日期范围内的每日记录。`}</p> : <><p className="statistics-footnote">已返回 {daily.length} 条日期记录 · {daily[0].date} 至 {daily[daily.length - 1].date}</p><ul className="account-daily-list">{daily.map((day) => <li key={day.date}><time>{day.date}</time><span className="account-daily-bar" aria-hidden="true"><i style={{ width: `${maxDaily > 0 ? Math.max(0, day.tokens / maxDaily * 100) : 0}%` }} /></span><span title={formatCount(day.tokens)}>{formatTokens(day.tokens)}</span></li>)}</ul></>}
    </section>
  </section>;
}

export function AccountStatisticsContent({ statistics, loading, refreshing, error, selectedPeriod = 'day', onPeriodChange, onRetry, dateRange = emptyDateRange, onDateRangeChange = () => {} }: Props) {
  const ready = statistics?.status === 'ready';
  const summary = ready ? statistics.summary : null;
  const busy = loading || refreshing;
  const failure = error || (statistics?.status === 'error' ? statistics.message || '读取服务端统计失败，请重试。' : null);
  const statusMessage = ready
    ? loading ? '正在更新服务端统计…' : statistics.message
    : busy ? loading ? '正在读取服务端统计…' : '暂无本地缓存，正在后台获取服务端统计…' : statistics?.message || '暂无服务端统计缓存。';
  return <>
    {failure ? <StatisticsErrorNotice message={`更新失败：${failure}${ready ? ' 当前显示本地缓存。' : ''}`} busy={busy} onRetry={onRetry} /> : statusMessage && <p className="statistics-notice" role="status">{statusMessage}</p>}
    <ActivityStatisticsSummary statistics={summary} />
    <StatisticsPeriodSwitch selectedPeriod={selectedPeriod} onPeriodChange={onPeriodChange} />
    {selectedPeriod === 'all' && <StatisticsDateRangeFilter value={dateRange} onChange={onDateRangeChange} />}
    <AccountPeriodStatistics statistics={ready ? statistics : null} selectedPeriod={selectedPeriod} dateRange={dateRange} />
    <div className="account-statistics-timestamps">
      <div className="statistics-updated">服务端更新于 {fullTime(ready ? statistics.serviceUpdatedAt : null)}</div>
      <div className="statistics-updated">采集于 {fullTime(ready ? statistics.updatedAt : null)}</div>
    </div>
  </>;
}

export function AccountStatisticsView({ source, store }: { source: Exclude<CodexStatisticsSource, 'local'>; store: ReturnType<typeof createAccountStatisticsStore> }) {
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  const [selectedPeriod, setSelectedPeriod] = useState<TokenPeriod['period']>('day');
  const [dateRange, setDateRange] = useState<StatisticsDateRange>(emptyDateRange);
  return <AccountStatisticsContent {...state} dateRange={dateRange} onDateRangeChange={setDateRange} selectedPeriod={selectedPeriod} onPeriodChange={setSelectedPeriod} onRetry={() => { void store.refresh(source, 'manual'); }} />;
}
