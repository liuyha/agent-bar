export function formatCountdown(iso: string | null, now = Date.now()): string {
  if (!iso) return '时间未知';
  const delta = Date.parse(iso) - now;
  if (!Number.isFinite(delta)) return '时间未知';
  if (delta <= 0) return '等待刷新';
  const minutes = Math.ceil(delta / 60_000);
  if (minutes < 60) return `${minutes} 分钟`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} 小时${minutes % 60 ? ` ${minutes % 60} 分钟` : ''}`;
  const days = Math.floor(hours / 24);
  return `${days} 天${hours % 24 ? ` ${hours % 24} 小时` : ''}`;
}

export function formatTime(iso: string): string {
  const time = new Date(iso);
  if (!Number.isFinite(time.getTime())) return '—';
  return time.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false });
}

export function formatTokens(value: number): string {
  if (!Number.isFinite(value) || value < 0) return '—';
  const units = ['', 'K', 'M', 'B'];
  let unit = 0;
  let scaled = value;
  while (scaled >= 1000 && unit < units.length - 1) {
    scaled /= 1000;
    unit += 1;
  }
  let rounded = Math.round(scaled * 100) / 100;
  // Rounding 999.995K/M must promote the result to the next unit.
  if (rounded >= 1000 && unit < units.length - 1) {
    rounded /= 1000;
    unit += 1;
  }
  return unit === 0 ? formatCount(value) : `${rounded}${units[unit]}`;
}

export function formatCount(value: number): string {
  return Number.isFinite(value) && value >= 0
    ? value.toLocaleString('zh-CN', { maximumFractionDigits: 0 })
    : '—';
}

// Fixed approximation for estimates, not a live or historical exchange rate.
// Keep model prices and cached totals in USD; convert only for display.
export const USD_TO_CNY_ESTIMATE_RATE = 7;

export function formatEstimatedCostCny(valueUsd: number | null): string {
  if (valueUsd === null || !Number.isFinite(valueUsd) || valueUsd < 0) return '暂无法估算';
  const valueCny = valueUsd * USD_TO_CNY_ESTIMATE_RATE;
  if (!Number.isFinite(valueCny)) return '暂无法估算';
  if (valueCny > 0 && valueCny < 0.01) return '< ¥0.01';
  return `¥${valueCny.toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

export function formatPeriodRange(startAt: string, endAt: string, includeYear = false): string {
  const start = new Date(startAt);
  const end = new Date(endAt);
  if (!Number.isFinite(start.getTime()) || !Number.isFinite(end.getTime())) return '日期未知';
  const year = includeYear || start.getFullYear() !== end.getFullYear() ? 'numeric' : undefined;
  const format = (date: Date) => date.toLocaleDateString('zh-CN', { year, month: 'numeric', day: 'numeric' });
  return start.toDateString() === end.toDateString() ? `${format(start)} 至今` : `${format(start)} – ${format(end)}`;
}
