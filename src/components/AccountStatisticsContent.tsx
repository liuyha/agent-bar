import { useState, useSyncExternalStore } from 'react';
import { AlertCircle, ExternalLink, LoaderCircle, RefreshCw } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { createAccountStatisticsStore, formatDuration, type AccountStatisticsState } from '../lib/accountStatistics';
import { accountStatisticsPeriod } from '../lib/accountStatisticsPeriods';
import { openCodexUsageWeb } from '../lib/api';
import { formatCount, formatTokens } from '../lib/format';
import { periodLabels } from '../lib/statisticsPeriods';
import { StatisticsPeriodSwitch } from './StatisticsPeriodSwitch';
import type { AccountUsageSnapshot, CodexStatisticsSource, TokenPeriod, WebUsageSnapshot } from '../types';

function fullTime(value: string | null): string {
  if (!value || !Number.isFinite(Date.parse(value))) return '—';
  if (/^\d{4}-\d{2}-\d{2}$/.test(value)) return value;
  return new Date(value).toLocaleString('zh-CN', { hour12: false });
}

function number(value: number | null, suffix = ''): string {
  return value === null || !Number.isFinite(value) ? '—' : `${value.toLocaleString('zh-CN', { maximumFractionDigits: 4 })}${suffix}`;
}

function WebStatistics({ web }: { web: WebUsageSnapshot | null }) {
  if (!web || web.status !== 'ready') return <p className="statistics-notice" role={web?.status === 'error' ? 'alert' : 'status'}>{web?.message || '尚未读取到网页补充数据。连接用量网页后，返回并刷新。'}</p>;
  return <>
    {web.message && <p className="statistics-notice" role="status">{web.message}</p>}
    <p className="statistics-account">网页账号：{web.account || '—'}</p>
    <dl className="account-statistics-metrics web-statistics-metrics">
      <div><dt>剩余 Credits</dt><dd>{number(web.creditsRemaining)}</dd></div>
      <div><dt>代码审查剩余额度</dt><dd>{number(web.codeReviewRemainingPercent, '%')}</dd></div>
    </dl>
    <details className="account-statistics-details"><summary>网页用量明细 · {web.usageUnit || '单位未提供'}</summary>
      {web.usageBreakdown === null ? <p className="statistics-footnote">网页未提供用量明细。</p> : web.usageBreakdown.length === 0 ? <p className="statistics-footnote">网页返回的用量明细为空。</p> : <ul className="web-statistics-list">{web.usageBreakdown.map((day, index) => <li key={`${day.date}:${index}`}><time>{day.date}</time>{day.amounts.length ? <dl>{day.amounts.map((item, itemIndex) => <div key={`${item.service}:${itemIndex}`}><dt>{item.service || '服务未提供'}</dt><dd>{number(item.amount)}{web.usageUnit && <small> {web.usageUnit}</small>}</dd></div>)}</dl> : <span>未提供数值</span>}</li>)}</ul>}
    </details>
    <details className="account-statistics-details"><summary>Credits 记录</summary>
      {web.creditEvents === null ? <p className="statistics-footnote">网页未提供 Credits 记录。</p> : web.creditEvents.length === 0 ? <p className="statistics-footnote">网页返回的 Credits 记录为空。</p> : <ul className="web-statistics-list">{web.creditEvents.map((event, index) => <li key={`${event.date}:${index}`}><time>{event.date}</time><dl><div><dt>{event.service || '服务未提供'}</dt><dd>{number(event.credits)}<small> Credits</small></dd></div></dl></li>)}</ul>}
    </details>
    <p className="statistics-footnote">网页数据单独展示，不随上方时段切换，单位以页面返回值为准。</p>
    <div className="statistics-updated">网页采集于 {fullTime(web.updatedAt)}</div>
  </>;
}

interface Props extends AccountStatisticsState {
  selectedPeriod?: TokenPeriod['period'];
  onPeriodChange: (period: TokenPeriod['period']) => void;
  webEnabled: boolean;
  onRetry: () => void;
  onOpenWeb: () => void;
  openingWeb?: boolean;
  connectionError?: string | null;
}

function AccountPeriodStatistics({ statistics, selectedPeriod }: { statistics: AccountUsageSnapshot; selectedPeriod: TokenPeriod['period'] }) {
  const period = accountStatisticsPeriod(statistics, selectedPeriod);
  const daily = period.dailyUsage;
  const maxDaily = daily?.reduce((maximum, day) => Math.max(maximum, day.tokens), 0) ?? 0;
  const all = selectedPeriod === 'all';
  return <section className="statistics-period account-period-statistics" aria-label={`${periodLabels[selectedPeriod]}服务端 Token 统计`}>
    <div className="statistics-period-heading"><h3>{periodLabels[selectedPeriod]}</h3><span>{all ? '覆盖范围以服务端为准' : period.startDate === period.endDate ? period.endDate : `${period.startDate} – ${period.endDate}`}</span></div>
    <dl className="account-statistics-metrics account-period-metrics">
      <div><dt>{all ? '累计 Token' : 'Token 合计（已返回）'}</dt><dd title={period.totalTokens === null ? undefined : formatCount(period.totalTokens)}>{period.totalTokens === null ? '—' : formatTokens(period.totalTokens)}</dd></div>
      <div><dt>{all ? '单日峰值 Token' : '单日峰值（已返回）'}</dt><dd title={period.peakDailyTokens === null ? undefined : formatCount(period.peakDailyTokens)}>{period.peakDailyTokens === null ? '—' : formatTokens(period.peakDailyTokens)}</dd></div>
    </dl>
    <section className="account-daily-statistics" aria-label="服务端每日 Token"><h3>每日 Token</h3>
      {daily === null ? <p className="statistics-footnote">服务端未提供每日 Token。</p> : daily.length === 0 ? <p className="statistics-footnote">{statistics.dailyUsage?.length === 0 ? '服务端返回的每日记录为空。' : `服务端尚未返回${periodLabels[selectedPeriod]}日期范围内的每日记录。`}</p> : <><p className="statistics-footnote">已返回 {daily.length} 条日期记录 · {daily[0].date} 至 {daily[daily.length - 1].date}</p><ul className="account-daily-list">{daily.map((day) => <li key={day.date}><time>{day.date}</time><span className="account-daily-bar" aria-hidden="true"><i style={{ width: `${maxDaily > 0 ? Math.max(0, day.tokens / maxDaily * 100) : 0}%` }} /></span><span title={formatCount(day.tokens)}>{formatTokens(day.tokens)}</span></li>)}</ul></>}
    </section>
    <p className="statistics-footnote">{all ? '累计与峰值采用服务端汇总；每日记录可能仅覆盖部分历史，不能据此还原全部用量。' : '合计与峰值仅根据所选时段已返回的每日记录计算，可能不完整。'}未返回的日期不计为零，缺失数据以 — 表示。</p>
  </section>;
}

export function AccountStatisticsContent({ statistics, loading, refreshing, error, selectedPeriod = 'day', onPeriodChange, webEnabled, onRetry, onOpenWeb, openingWeb = false, connectionError }: Props) {
  const ready = statistics?.status === 'ready';
  const summary = ready ? statistics.summary : null;
  const collected = ready && Boolean(statistics.updatedAt && Number.isFinite(Date.parse(statistics.updatedAt)));
  return <>
    <div className="account-statistics-toolbar"><span className="statistics-source-badge">实际来源：{collected ? '服务端' : '—'}</span><Button variant="outline" size="icon" type="button" aria-label="刷新服务端统计" aria-busy={refreshing} title={refreshing && !loading ? '后台刷新中，点击显示加载状态' : '刷新服务端统计'} disabled={loading} onClick={onRetry}><RefreshCw size={13} className={refreshing ? 'spin' : undefined} /></Button></div>
    {loading ? <div className="statistics-state" role="status"><LoaderCircle size={22} className="spin" /><p>正在读取服务端统计…</p></div> : !ready ? refreshing ? <p className="statistics-notice" role="status">暂无本地缓存，正在后台获取服务端统计…</p> : <div className="statistics-state" role={error || statistics?.status === 'error' ? 'alert' : 'status'}><AlertCircle size={22} /><p>{error || statistics?.message || '暂无服务端统计缓存。'}</p><Button type="button" variant="outline" className="mt-1.5 text-[10px]" onClick={onRetry}><RefreshCw size={13} />重新读取</Button></div> : <>
      {error && <p className="statistics-notice" role="alert">更新失败：{error} 当前显示本地缓存。</p>}
      {statistics.message && <p className="statistics-notice" role="status">{statistics.message}</p>}
      <StatisticsPeriodSwitch selectedPeriod={selectedPeriod} onPeriodChange={onPeriodChange} />
      <AccountPeriodStatistics statistics={statistics} selectedPeriod={selectedPeriod} />
      <p className="statistics-footnote">按本地日历选择日期，周一为每周起点，本年从 1 月 1 日起；服务端日期原样匹配，不转换时区。日界线与本机记录可能不同。</p>
      <p className="statistics-account">账号：{statistics.account || statistics.accountId || '—'}</p>
      {statistics.account && statistics.accountId && <p className="statistics-account">账号 ID：{statistics.accountId}</p>}
      <details className="account-statistics-details"><summary>账号活动概览（不随时段切换）</summary><dl className="account-statistics-metrics account-activity-metrics">
        <div><dt>最长任务时长</dt><dd>{formatDuration(summary?.longestRunningTurnSec ?? null)}</dd></div>
        <div><dt>当前连续活跃</dt><dd>{number(summary?.currentStreakDays ?? null, ' 天')}</dd></div>
        <div><dt>最长连续活跃</dt><dd>{number(summary?.longestStreakDays ?? null, ' 天')}</dd></div>
      </dl></details>
      <div className="statistics-updated">服务端更新于 {fullTime(statistics.serviceUpdatedAt)}</div>
      <div className="statistics-updated">采集于 {fullTime(statistics.updatedAt)}</div>
    </>}
    {webEnabled && <section className="web-statistics" aria-label="可选网页补充"><h3>网页补充</h3><div className="web-statistics-actions"><Button variant="outline" className="px-2 text-[10px]" type="button" disabled={openingWeb} onClick={onOpenWeb}>{openingWeb ? <LoaderCircle size={12} className="spin" /> : <ExternalLink size={12} />}连接用量网页</Button><Button variant="outline" className="px-2 text-[10px]" type="button" disabled={loading} onClick={onRetry}><RefreshCw size={12} className={refreshing ? 'spin' : undefined} />返回后刷新</Button></div><p className="statistics-footnote">网页登录仅保留至 AgentBar 退出；重新启动后需要再次连接。</p>{connectionError && <p className="statistics-notice" role="alert">{connectionError}</p>}<WebStatistics web={loading ? null : statistics?.web ?? null} /></section>}
  </>;
}

export function AccountStatisticsView({ source, webEnabled, store }: { source: Exclude<CodexStatisticsSource, 'local'>; webEnabled: boolean; store: ReturnType<typeof createAccountStatisticsStore> }) {
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  const [openingWeb, setOpeningWeb] = useState(false);
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const [selectedPeriod, setSelectedPeriod] = useState<TokenPeriod['period']>('day');
  async function openWeb() {
    setConnectionError(null);
    setOpeningWeb(true);
    try { await openCodexUsageWeb(); }
    catch (reason) { setConnectionError(reason instanceof Error ? reason.message : String(reason)); }
    finally { setOpeningWeb(false); }
  }
  return <AccountStatisticsContent {...state} selectedPeriod={selectedPeriod} onPeriodChange={setSelectedPeriod} webEnabled={webEnabled} openingWeb={openingWeb} connectionError={connectionError} onRetry={() => { void store.refresh(source, 'manual'); }} onOpenWeb={() => { void openWeb(); }} />;
}
