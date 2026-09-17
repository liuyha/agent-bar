import { describe, expect, it } from 'vitest';
import { formatCount, formatCountdown, formatPeriodRange, formatTokens, formatUsd } from './format';

describe('reset countdown', () => {
  const now = Date.parse('2026-09-16T00:00:00Z');
  it('handles expired and invalid timestamps without negative output', () => {
    expect(formatCountdown('2026-09-15T00:00:00Z', now)).toBe('等待刷新');
    expect(formatCountdown('invalid', now)).toBe('时间未知');
    expect(formatCountdown(null, now)).toBe('时间未知');
  });
  it('rounds partial minutes up and formats longer windows', () => {
    expect(formatCountdown(new Date(now + 1000).toISOString(), now)).toBe('1 分钟');
    expect(formatCountdown(new Date(now + 90 * 60_000).toISOString(), now)).toBe('1 小时 30 分钟');
    expect(formatCountdown(new Date(now + 49 * 3600_000).toISOString(), now)).toBe('2 天 1 小时');
  });
});

describe('statistics date ranges', () => {
  const localDate = (year: number, month: number, day: number) => new Date(year, month - 1, day, 12).toISOString();
  it('shows years for annual and historical ranges, including a single day', () => {
    expect(formatPeriodRange(localDate(2026, 1, 1), localDate(2026, 9, 17), true)).toBe('2026/1/1 – 2026/9/17');
    expect(formatPeriodRange(localDate(2024, 1, 1), localDate(2026, 9, 17), true)).toBe('2024/1/1 – 2026/9/17');
    expect(formatPeriodRange(localDate(2026, 9, 17), localDate(2026, 9, 17), true)).toBe('2026/9/17 至今');
  });
  it('keeps short ranges compact but disambiguates weeks spanning a year boundary', () => {
    expect(formatPeriodRange(localDate(2026, 9, 1), localDate(2026, 9, 17))).toBe('9/1 – 9/17');
    expect(formatPeriodRange(localDate(2025, 12, 29), localDate(2026, 1, 2))).toBe('2025/12/29 – 2026/1/2');
    expect(formatPeriodRange('invalid', localDate(2026, 9, 17), true)).toBe('日期未知');
  });
});

describe('token and estimated cost formatting', () => {
  it.each([
    [0, '0'], [1, '1'], [999, '999'],
    [1000, '1K'], [1024, '1.02K'], [1200, '1.2K'], [123456, '123.46K'],
    [1000000, '1M'], [1234567, '1.23M'],
    [1000000000, '1B'], [1200000000, '1.2B'], [1234567890, '1.23B'],
    [1000000000000, '1000B'],
  ])('formats %s tokens with decimal units and no trailing zeros as %s', (value, expected) => {
    expect(formatTokens(value)).toBe(expected);
  });
  it.each([
    [999994, '999.99K'], [999995, '1M'], [999999, '1M'],
    [999994999, '999.99M'], [999995000, '1B'], [999999999, '1B'],
  ])('promotes rounded values across unit boundaries: %s → %s', (value, expected) => {
    expect(formatTokens(value)).toBe(expected);
  });
  it.each([Number.NaN, Infinity, -Infinity, -1])('rejects invalid counts: %s', (value) => {
    expect(formatTokens(value)).toBe('—');
    expect(formatCount(value)).toBe('—');
  });
  it('keeps request counts, turns and exact token titles unabridged', () => {
    expect(formatCount(0)).toBe('0');
    expect(formatCount(1234567)).toBe('1,234,567');
    expect(formatCount(1234567890)).toBe('1,234,567,890');
  });
  it('distinguishes unknown, zero and sub-cent USD amounts', () => {
    expect(formatUsd(null)).toBe('暂无法估算');
    expect(formatUsd(Number.NaN)).toBe('暂无法估算');
    expect(formatUsd(0)).toBe('US$0.00');
    expect(formatUsd(0.002)).toBe('< US$0.01');
    expect(formatUsd(1234.567)).toBe('US$1,234.57');
  });
});
