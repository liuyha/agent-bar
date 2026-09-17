import { AlertCircle, ChevronLeft, ChevronRight, Clock3, RefreshCw } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Progress } from '@/components/ui/progress';
import { formatCountdown, formatTime } from '../lib/format';
import type { ProviderUsage } from '../types';

interface ProviderCardProps {
  provider: ProviderUsage;
  now: number;
  active?: boolean;
  onShowStatistics?: (anchor: HTMLElement, focus?: boolean) => void;
  onLeaveStatistics?: (insideWindow: boolean) => void;
  statisticsSide?: 'left' | 'right' | null;
  detachedStatistics?: boolean;
  onRefresh?: () => void;
  refreshing?: boolean;
  refreshDisabled?: boolean;
  refreshError?: string | null;
}

export function ProviderCard({ provider, now, active = false, onShowStatistics, onLeaveStatistics, statisticsSide, detachedStatistics = false, onRefresh, refreshing = false, refreshDisabled = false, refreshError }: ProviderCardProps) {
  const windows = provider.status === 'ready' ? provider.windows.filter((window) => Number.isFinite(window.usedPercent)) : [];
  const statusLabel = provider.status === 'ready' ? '已连接' : provider.status === 'error' ? '读取失败' : '待获取';
  const message = provider.message || (provider.status === 'error'
    ? '暂时无法读取账号用量，请稍后刷新。'
    : provider.status === 'unavailable'
      ? '尚未获取本机账号，请先登录对应服务后刷新。'
      : windows.length === 0 ? '当前账号未返回可用的用量信息。' : null);

  return (
    <article className={`provider-card provider-${provider.id}${active ? ' provider-card-active' : ''}`} aria-label={`${provider.name} 账号用量`} onMouseEnter={(event) => onShowStatistics?.(event.currentTarget)} onMouseLeave={(event) => onLeaveStatistics?.(event.relatedTarget instanceof Node && document.documentElement.contains(event.relatedTarget))} onFocus={(event) => { if (!detachedStatistics) onShowStatistics?.(event.currentTarget); }}>
      <div className="provider-heading">
        <span className="provider-icon" role="img" aria-label={provider.name} title={provider.name} />
        <div className="provider-identity">
          {provider.account && <span className="provider-account" title={provider.account}>{provider.account}</span>}
          {provider.plan && <span className="provider-plan" title={provider.plan}>{provider.plan}</span>}
        </div>
        <span className={`provider-status status-${provider.status}`}>{statusLabel}</span>
        {onRefresh && <Button type="button" variant="ghost" size="icon" className="shrink-0" aria-label={`刷新 ${provider.name} 用量`} title={`刷新 ${provider.name} 用量`} disabled={refreshing || refreshDisabled} onClick={onRefresh}><RefreshCw size={13} className={refreshing ? 'spin' : undefined} /></Button>}
      </div>
      {refreshError && <div className="provider-message provider-message-error" role="alert"><AlertCircle size={13} aria-hidden="true" /><span>{refreshError}</span></div>}
      {message && <div className={`provider-message${provider.status === 'error' ? ' provider-message-error' : ''}`} role={provider.status === 'error' ? 'alert' : 'status'}>{provider.status === 'error' && <AlertCircle size={13} aria-hidden="true" />}<span>{message}</span></div>}
      {windows.length > 0 && <div className="usage-windows">
        {windows.map((window) => {
          const label = window.label.replace(/GPT-5\.3-Codex-Spark/gi, 'Codex-Spark');
          const usedPercent = Math.min(100, Math.max(0, window.usedPercent));
          const remainingPercent = 100 - usedPercent;
          const displayPercent = Math.round(remainingPercent);
          const countdown = formatCountdown(window.resetsAt, now);
          return (
            <div className="usage-window" key={window.label}>
              <div className="usage-label-row">
                <span className="usage-title"><span className="usage-label">{label}</span>{' '}<strong className="usage-number">{displayPercent}% 剩余</strong></span>
                <span className="usage-reset">{countdown === '等待刷新' || countdown === '时间未知' ? countdown : `${countdown}后重置`}</span>
              </div>
              <Progress
                value={remainingPercent}
                className={usedPercent >= 80 ? 'progress-high' : undefined}
                indicatorClassName={usedPercent >= 80 ? 'bg-[var(--warning)]' : undefined}
                aria-label={`${provider.name} ${label}剩余`}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={displayPercent}
              />
            </div>
          );
        })}
      </div>}
      {onRefresh && <div className="provider-update-status" aria-live="polite"><Clock3 size={11} aria-hidden="true" /><span>{refreshing ? `正在刷新 ${provider.name}…` : provider.updatedAt ? `更新于 ${formatTime(provider.updatedAt)}` : '尚未获取用量'}</span></div>}
      {onShowStatistics && <Button type="button" variant="ghost" className={`mt-3 h-auto w-full justify-between rounded-none border-0 border-t border-[var(--line)] px-0 pb-0 pt-[9px] text-left text-[10px] font-normal hover:bg-transparent ${active ? 'text-[var(--text)]' : 'text-[var(--muted)]'}`} aria-expanded={active} aria-haspopup={detachedStatistics ? 'dialog' : undefined} aria-controls={!detachedStatistics && active ? 'token-statistics' : undefined}
        onClick={(event) => { const anchor = event.currentTarget.closest('article'); if (anchor) onShowStatistics(anchor, true); }}
        onKeyDown={(event) => { if (event.key !== (statisticsSide === 'left' ? 'ArrowLeft' : 'ArrowRight')) return; event.preventDefault(); const anchor = event.currentTarget.closest('article'); if (anchor) onShowStatistics(anchor, true); }}><span>查看 Token、金额与交互统计</span>{active && statisticsSide === 'left' ? <ChevronLeft size={13} aria-hidden="true" /> : <ChevronRight size={13} aria-hidden="true" />}</Button>}
    </article>
  );
}
