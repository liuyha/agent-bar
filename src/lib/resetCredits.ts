import type { ResetCredit, ResetCredits } from '../types';

function expiry(credit: ResetCredit): number {
  const timestamp = credit.expiresAt ? Date.parse(credit.expiresAt) : Number.NaN;
  return Number.isFinite(timestamp) ? timestamp : Infinity;
}

export function availableResetCredits(value: ResetCredits, now: number): { remaining: number | null; credits: ResetCredit[] | null } {
  const remaining = value.remaining !== null && Number.isSafeInteger(value.remaining) && value.remaining >= 0
    ? value.remaining : null;
  if (value.credits === null) return { remaining, credits: null };
  const valid = value.credits.filter((credit) => Number.isSafeInteger(credit.remaining) && credit.remaining > 0);
  const expired = valid.filter((credit) => expiry(credit) <= now).reduce((sum, credit) => sum + credit.remaining, 0);
  return {
    remaining: remaining === null ? null : Math.max(0, remaining - expired),
    credits: valid.filter((credit) => expiry(credit) > now).sort((a, b) => expiry(a) - expiry(b)),
  };
}

export function formatCreditExpiry(iso: string | null): string {
  if (!iso || !Number.isFinite(Date.parse(iso))) return '过期时间未知';
  return `${new Date(iso).toLocaleString('zh-CN', {
    year: 'numeric', month: '2-digit', day: '2-digit',
    hour: '2-digit', minute: '2-digit', hour12: false,
  })} 到期`;
}
