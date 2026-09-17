import { AlertCircle } from 'lucide-react';
import type { ProviderId, ProviderUsage } from '../types';

interface ConnectionNoticeProps {
  providers: ProviderUsage[];
  errors: Partial<Record<ProviderId, string | null>>;
}

export function ConnectionNotice({ providers, errors }: ConnectionNoticeProps) {
  const failures = providers.flatMap((provider) => {
    const message = errors[provider.id] || (provider.status === 'error'
      ? provider.message || '暂时无法读取账号用量，请稍后刷新。'
      : null);
    return message ? [{ provider, message }] : [];
  });
  if (failures.length === 0) return null;

  const hasPreviousUsage = failures.some(({ provider }) => provider.windows.some((window) => Number.isFinite(window.usedPercent)));
  return <div className="error-notice connection-notice" role="alert">
    <AlertCircle size={15} aria-hidden="true" />
    <div>
      <strong>连接失败</strong>
      {failures.map(({ provider, message }) => <p key={provider.id}>{provider.name}：{message}</p>)}
      {hasPreviousUsage && <p className="connection-notice-hint">保留上次成功获取的用量，可点击刷新重试。</p>}
    </div>
  </div>;
}
