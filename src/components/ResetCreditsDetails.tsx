import { ChevronDown, RotateCcw } from 'lucide-react';
import { formatCount } from '../lib/format';
import { availableResetCredits, formatCreditExpiry } from '../lib/resetCredits';
import type { ResetCredits } from '../types';

export function ResetCreditsDetails({ value, now }: { value: ResetCredits | null | undefined; now: number }) {
  const { remaining, credits } = value ? availableResetCredits(value, now) : { remaining: null, credits: null };
  const partialList = remaining !== null && credits !== null && credits.length > 0
    && credits.reduce((sum, credit) => sum + credit.remaining, 0) < remaining;
  return (
    <details className="reset-credits" onKeyDown={(event) => {
      if (event.key !== 'Escape' || event.nativeEvent.isComposing || !event.currentTarget.open) return;
      event.preventDefault();
      event.stopPropagation();
      event.currentTarget.open = false;
      event.currentTarget.querySelector('summary')?.focus();
    }}>
      <summary className="reset-credits-toggle" aria-label={`Codex 重置剩余${remaining === null ? '数量未知' : ` ${remaining} 次`}，查看重置次数列表`}>
        <span className="reset-credits-label"><RotateCcw size={12} aria-hidden="true" />重置剩余</span>
        <span className="reset-credits-count">{remaining === null ? '暂不可用' : <><strong>{formatCount(remaining)}</strong> 次</>}<ChevronDown size={12} aria-hidden="true" /></span>
      </summary>
      <div className="reset-credits-content">
        <div className="reset-credits-heading">重置次数列表<span>过期时间</span></div>
        {credits && credits.length > 0 ? <ul className="reset-credits-list" aria-label="可用重置次数">
          {credits.map((credit) => <li key={credit.id}>
            <span className="reset-credit-quantity">{formatCount(credit.remaining)} 次</span>
            <span className="reset-credit-expiry">{formatCreditExpiry(credit.expiresAt)}</span>
          </li>)}
        </ul> : <p className="reset-credits-notice">{credits === null
          ? value?.message || '暂未获取重置次数明细，请刷新 Codex 用量。'
          : remaining === 0 ? '暂无可用重置次数。' : '暂未获取可用次数的过期时间。'}</p>}
        {partialList && <p className="reset-credits-notice">部分重置次数未返回过期时间。</p>}
        {value?.message && credits !== null && <p className="reset-credits-notice">{value.message}</p>}
      </div>
    </details>
  );
}
