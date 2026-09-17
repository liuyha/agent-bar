import { AlertCircle, ChevronRight, Clock3, RefreshCw } from 'lucide-react';
import { formatCountdown, formatTime } from '../lib/format';
import type { ProviderUsage } from '../types';

interface ProviderCardProps {
  provider: ProviderUsage;
  now: number;
  active?: boolean;
  onShowStatistics?: () => void;
  onRefresh?: () => void;
  refreshing?: boolean;
  refreshDisabled?: boolean;
  refreshError?: string | null;
}

export function ProviderCard({ provider, now, active = false, onShowStatistics, onRefresh, refreshing = false, refreshDisabled = false, refreshError }: ProviderCardProps) {
  const windows = provider.status === 'ready' ? provider.windows.filter((window) => Number.isFinite(window.usedPercent)) : [];
  const statusLabel = provider.status === 'ready' ? '已连接' : provider.status === 'error' ? '读取失败' : '待获取';
  const message = provider.message || (provider.status === 'error'
    ? '暂时无法读取账号用量，请稍后刷新。'
    : provider.status === 'unavailable'
      ? '尚未获取本机账号，请先登录对应服务后刷新。'
      : windows.length === 0 ? '当前账号未返回可用的用量信息。' : null);

  return (
    <article className={`provider-card provider-${provider.id}${active ? ' provider-card-active' : ''}`} aria-label={`${provider.name} 账号用量`} onMouseEnter={onShowStatistics} onFocus={onShowStatistics}>
      <div className="provider-heading">
        <span className="provider-icon" role="img" aria-label={provider.name} title={provider.name} />
        <div className="provider-identity">
          {provider.account && <span className="provider-account" title={provider.account}>{provider.account}</span>}
          {provider.plan && <span className="provider-plan" title={provider.plan}>{provider.plan}</span>}
        </div>
        <span className={`provider-status status-${provider.status}`}>{statusLabel}</span>
        {onRefresh && <button type="button" className="icon-button provider-refresh" aria-label={`刷新 ${provider.name} 用量`} title={`刷新 ${provider.name} 用量`} disabled={refreshing || refreshDisabled} onClick={onRefresh}><RefreshCw size={13} className={refreshing ? 'spin' : undefined} /></button>}
      </div>
      {refreshError && <div className="provider-message provider-message-error" role="alert"><AlertCircle size={13} aria-hidden="true" /><span>{refreshError}</span></div>}
      {message && <div className={`provider-message${provider.status === 'error' ? ' provider-message-error' : ''}`} role={provider.status === 'error' ? 'alert' : 'status'}>{provider.status === 'error' && <AlertCircle size={13} aria-hidden="true" />}<span>{message}</span></div>}
      {windows.length > 0 && <div className="usage-windows">
        {windows.map((window) => {
          const usedPercent = Math.min(100, Math.max(0, window.usedPercent));
          const remainingPercent = 100 - usedPercent;
          const displayPercent = Math.round(remainingPercent);
          const countdown = formatCountdown(window.resetsAt, now);
          return (
            <div className="usage-window" key={window.label}>
              <div className="usage-label-row">
                <span className="usage-title"><span className="usage-label">{window.label}</span>{' '}<strong className="usage-number">{displayPercent}% 剩余</strong></span>
                <span className="usage-reset">{countdown === '等待刷新' || countdown === '时间未知' ? countdown : `${countdown}后重置`}</span>
              </div>
              <div
                className={`progress-track${usedPercent >= 80 ? ' progress-high' : ''}`}
                role="progressbar"
                aria-label={`${provider.name} ${window.label}剩余`}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={displayPercent}
              >
                <span style={{ width: `${remainingPercent}%` }} />
              </div>
            </div>
          );
        })}
      </div>}
      {onRefresh && <div className="provider-update-status" aria-live="polite"><Clock3 size={11} aria-hidden="true" /><span>{refreshing ? `正在刷新 ${provider.name}…` : provider.updatedAt ? `更新于 ${formatTime(provider.updatedAt)}` : '尚未获取用量'}</span></div>}
      {onShowStatistics && <button type="button" className="statistics-trigger" aria-expanded={active} aria-controls={active ? 'token-statistics' : undefined} onClick={onShowStatistics}><span>查看 Token、金额与交互统计</span><ChevronRight size={13} aria-hidden="true" /></button>}
    </article>
  );
}
