import { formatDuration } from '../lib/accountStatistics';
import type { ActivityStatistics } from '../types';

function days(value: number | null | undefined): string {
  return value == null || !Number.isFinite(value) || value < 0 ? '—' : `${value.toLocaleString('zh-CN', { maximumFractionDigits: 4 })} 天`;
}

export function ActivityStatisticsSummary({ statistics, source = 'server' }: {
  statistics: ActivityStatistics | null | undefined;
  source?: 'local' | 'server';
}) {
  return <dl className="account-statistics-metrics account-activity-metrics" aria-label={source === 'local' ? '本机活动概览' : '账号活动概览'}>
    <div><dt>最长任务时长</dt><dd>{formatDuration(statistics?.longestRunningTurnSec ?? null)}</dd></div>
    <div><dt>当前连续活跃</dt><dd>{days(statistics?.currentStreakDays)}</dd></div>
    <div><dt>最长连续活跃</dt><dd>{days(statistics?.longestStreakDays)}</dd></div>
  </dl>;
}
