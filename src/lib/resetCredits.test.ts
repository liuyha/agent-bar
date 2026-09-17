import { describe, expect, it } from 'vitest';
import { availableResetCredits, formatCreditExpiry } from './resetCredits';
import type { ResetCredits } from '../types';

const now = Date.parse('2026-09-17T04:00:00Z');
const base: ResetCredits = { remaining: 3, credits: null, updatedAt: null, message: null };

describe('available reset credits', () => {
  it('preserves the server count when the detail request fails and distinguishes unknown from zero', () => {
    expect(availableResetCredits(base, now)).toEqual({ remaining: 3, credits: null });
    expect(availableResetCredits({ ...base, remaining: null }, now).remaining).toBeNull();
    expect(availableResetCredits({ ...base, remaining: 0, credits: [] }, now)).toEqual({ remaining: 0, credits: [] });
  });

  it('expires cached credits at the exact boundary without waiting for a refresh, and sorts the rest by expiry', () => {
    const value: ResetCredits = { ...base, remaining: 5, credits: [
      { id: 'later', remaining: 1, expiresAt: '2026-10-01T00:00:00Z' },
      { id: 'unknown', remaining: 1, expiresAt: null },
      { id: 'expired', remaining: 2, expiresAt: '2026-09-17T04:00:00Z' },
      { id: 'soon', remaining: 1, expiresAt: '2026-09-18T00:00:00Z' },
    ] };
    const result = availableResetCredits(value, now);
    expect(result.remaining).toBe(3);
    expect(result.credits?.map((credit) => credit.id)).toEqual(['soon', 'later', 'unknown']);
    expect(value.credits?.[0].id).toBe('later');
    expect(availableResetCredits(value, Date.parse('2026-11-01T00:00:00Z')).remaining).toBe(1);
  });

  it('keeps unknown expiry unknown and never invents a count from incomplete details', () => {
    const result = availableResetCredits({ ...base, remaining: null, credits: [
      { id: 'unknown', remaining: 1, expiresAt: 'invalid' },
      { id: 'invalid', remaining: -1, expiresAt: null },
      { id: 'empty', remaining: 0, expiresAt: null },
    ] }, now);
    expect(result.remaining).toBeNull();
    expect(result.credits?.map((credit) => credit.id)).toEqual(['unknown']);
    expect(formatCreditExpiry('invalid')).toBe('过期时间未知');
    expect(formatCreditExpiry(null)).toBe('过期时间未知');
  });

  it('retains the authoritative total when only some credit details are returned', () => {
    const value: ResetCredits = { ...base, remaining: 8, credits: [
      { id: 'soon', remaining: 1, expiresAt: '2026-09-17T04:00:00Z' },
      { id: 'later', remaining: 1, expiresAt: '2026-10-01T00:00:00Z' },
    ] };
    expect(availableResetCredits(value, now).remaining).toBe(7);
    expect(availableResetCredits(value, now).credits).toHaveLength(1);
  });

  it('formats the full expiry date in the local timezone, including year and minute', () => {
    const local = new Date(2026, 8, 18, 12, 34).toISOString();
    expect(formatCreditExpiry(local)).toBe('2026/09/18 12:34 到期');
  });
});
