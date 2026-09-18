import { useId } from 'react';
import { Button } from './ui/button';
import { calendarDate, dateRangeError, emptyDateRange, hasDateRange, type StatisticsDateRange } from '../lib/statisticsDateRange';

export function StatisticsDateRangeFilter({ value, onChange }: {
  value: StatisticsDateRange;
  onChange: (range: StatisticsDateRange) => void;
}) {
  const id = useId();
  const error = dateRangeError(value);
  const today = calendarDate(new Date());
  return <div className="statistics-date-filter" role="group" aria-label="按日期范围筛选">
    <div className="statistics-date-fields">
      <label htmlFor={`${id}-start`}>开始日期<input id={`${id}-start`} type="date" value={value.startDate} max={value.endDate || today} aria-invalid={Boolean(error)} aria-describedby={error ? `${id}-error` : undefined} onChange={(event) => onChange({ ...value, startDate: event.target.value })} /></label>
      <span className="statistics-date-separator" aria-hidden="true">至</span>
      <label htmlFor={`${id}-end`}>结束日期<input id={`${id}-end`} type="date" value={value.endDate} min={value.startDate || undefined} max={today} aria-invalid={Boolean(error)} aria-describedby={error ? `${id}-error` : undefined} onChange={(event) => onChange({ ...value, endDate: event.target.value })} /></label>
      <Button type="button" variant="ghost" disabled={!hasDateRange(value)} onClick={() => onChange({ ...emptyDateRange })}>重置</Button>
    </div>
    {error && <p id={`${id}-error`} className="statistics-date-error" role="alert">{error}</p>}
  </div>;
}
