import { AlertCircle, RefreshCw } from 'lucide-react';
import { Button } from '@/components/ui/button';

export function StatisticsErrorNotice({ message, busy, onRetry }: { message: string; busy: boolean; onRetry: () => void }) {
  return <div className="statistics-notice statistics-error-notice" role="alert">
    <AlertCircle size={14} aria-hidden="true" />
    <p>{message}</p>
    <Button type="button" variant="outline" disabled={busy} aria-busy={busy} onClick={onRetry}>
      <RefreshCw size={12} className={busy ? 'spin' : undefined} />{busy ? '读取中…' : '重新读取'}
    </Button>
  </div>;
}
