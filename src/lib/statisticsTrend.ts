import type { AccountUsageSnapshot, TokenPeriod, TokenStatistics } from '../types';
import { accountStatisticsPeriod } from './accountStatisticsPeriods';
import { calendarDate, dateRangeError, emptyDateRange, hasDateRange, type StatisticsDateRange } from './statisticsDateRange';

export interface TrendPoint {
  key: string;
  label: string;
  tokens: number | null;
}

export interface TrendData {
  granularity: 'hour' | 'day';
  points: TrendPoint[];
  message?: string;
}

// Keep daily resolution even for long histories, but reject malformed/extreme ranges.
const maxPoints = 36_600;
const hourMs = 3_600_000;

function bucketStart(date: Date, granularity: TrendData['granularity']): Date {
  if (granularity === 'hour') {
    // Arithmetic preserves the offset of a repeated hour when daylight saving time ends.
    return new Date(date.getTime() - date.getMinutes() * 60_000 - date.getSeconds() * 1_000 - date.getMilliseconds());
  }
  const result = new Date(date);
  result.setHours(0, 0, 0, 0);
  return result;
}

function bucketKey(date: Date, granularity: TrendData['granularity']): string {
  return granularity === 'hour' ? date.toISOString() : calendarDate(date);
}

function hourLabel(date: Date): string {
  const offset = -date.getTimezoneOffset();
  const hours = Math.floor(Math.abs(offset) / 60).toString().padStart(2, '0');
  const minutes = (Math.abs(offset) % 60).toString().padStart(2, '0');
  return `${calendarDate(date)} ${date.getHours().toString().padStart(2, '0')}:00 UTC${offset >= 0 ? '+' : '-'}${hours}:${minutes}`;
}

function series(start: Date, end: Date, granularity: TrendData['granularity'], values: Map<string, number>, missing: number | null): TrendData {
  const result: TrendData = { granularity, points: [] };
  if (!Number.isFinite(start.getTime()) || !Number.isFinite(end.getTime()) || start > end) return result;
  const cursor = bucketStart(start, granularity);
  while (cursor <= end) {
    if (result.points.length === maxPoints) return { granularity, points: [], message: '日期范围过长，请缩小范围后查看趋势。' };
    const key = bucketKey(cursor, granularity);
    result.points.push({ key, label: granularity === 'hour' ? hourLabel(cursor) : key, tokens: values.get(key) ?? missing });
    if (granularity === 'hour') cursor.setTime(cursor.getTime() + hourMs);
    else cursor.setDate(cursor.getDate() + 1);
  }
  return result;
}

export function localStatisticsTrend(
  statistics: TokenStatistics | null,
  selectedPeriod: TokenPeriod['period'],
  range: StatisticsDateRange = emptyDateRange,
): TrendData {
  const granularity = selectedPeriod === 'day' ? 'hour' : 'day';
  const empty: TrendData = { granularity, points: [] };
  const now = new Date();
  const filtered = selectedPeriod === 'all' && hasDateRange(range);
  if (filtered && dateRangeError(range, now)) return empty;
  if (!statistics || statistics.status !== 'ready') return { ...empty, message: '暂无使用趋势数据。' };
  const buckets = granularity === 'hour' ? statistics.hourlyPeriods : statistics.dailyPeriods;
  if (!buckets) return { ...empty, message: `当前缓存缺少${granularity === 'hour' ? '小时' : '每日'}明细，请刷新使用统计。` };
  const period = statistics.periods.find((item) => item.period === selectedPeriod);
  if (!period) return { ...empty, message: '暂无所选时段的使用趋势。' };
  const start = filtered && range.startDate ? new Date(`${range.startDate}T00:00:00`) : new Date(period.startAt);
  const end = filtered && range.endDate ? new Date(`${range.endDate}T23:59:59.999`) : new Date(period.endAt);
  const collectedAt = new Date(statistics.updatedAt).getTime();
  if (!Number.isFinite(collectedAt)) return { ...empty, message: '统计更新时间无效，请刷新使用统计。' };
  end.setTime(Math.min(end.getTime(), collectedAt, now.getTime()));
  const values = new Map<string, number>();
  for (const bucket of buckets) {
    const timestamp = new Date(bucket.startAt);
    if (!Number.isFinite(timestamp.getTime()) || timestamp > end) continue;
    const key = bucketKey(bucketStart(timestamp, granularity), granularity);
    values.set(key, (values.get(key) ?? 0) + bucket.totalTokens);
  }
  return series(start, end, granularity, values, 0);
}

export function accountStatisticsTrend(
  snapshot: AccountUsageSnapshot | null,
  selectedPeriod: TokenPeriod['period'],
  range: StatisticsDateRange = emptyDateRange,
  now = new Date(),
): TrendData {
  const granularity = selectedPeriod === 'day' ? 'hour' : 'day';
  const empty: TrendData = { granularity, points: [] };
  if (selectedPeriod === 'all' && dateRangeError(range, now)) return empty;
  if (!snapshot || snapshot.status !== 'ready') return { ...empty, message: '暂无使用趋势数据。' };
  if (selectedPeriod === 'day') return { ...empty, message: '服务端暂未提供小时记录，无法展示昨日按小时趋势。' };
  const period = accountStatisticsPeriod(snapshot, selectedPeriod, now, range);
  const values = new Map<string, number>();
  for (const day of period.dailyUsage ?? []) {
    if (dateRangeError({ startDate: day.date, endDate: '' }, now)) continue;
    values.set(day.date, day.tokens);
  }
  const startDate = period.startDate ?? [...values.keys()].sort()[0];
  if (!startDate) return { ...empty, message: '服务端暂无每日使用记录。' };
  const result = series(new Date(`${startDate}T00:00:00`), new Date(`${period.endDate}T23:59:59.999`), 'day', values, null);
  if (!result.message && result.points.some((point) => point.tokens === null)) result.message = '缺失日期表示服务端未提供记录。';
  return result;
}
