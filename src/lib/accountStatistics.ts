import type { AccountUsageSnapshot, CodexStatisticsSource } from '../types';
import { getCachedCodexAccountStatistics, getCodexAccountStatistics } from './api';

export interface AccountStatisticsState {
  statistics: AccountUsageSnapshot | null;
  loading: boolean;
  refreshing: boolean;
  error: string | null;
}

export function formatDuration(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds) || seconds < 0) return '—';
  if (seconds < 60) return `${seconds} 秒`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} 分 ${seconds % 60} 秒`;
  return `${Math.floor(minutes / 60)} 小时 ${minutes % 60} 分`;
}

type RemoteSource = Exclude<CodexStatisticsSource, 'local'>;
type RefreshMode = 'background' | 'manual';

interface Request {
  source: RemoteSource;
  generation: number;
  fetchRequired: boolean;
  promise: Promise<void>;
}

// Each account lifecycle owns its store. Rust restores only successful snapshots whose
// authentication, source, and settings still match the current account context.
export function createAccountStatisticsStore(
  fetchStatistics = getCodexAccountStatistics,
  readCached = getCachedCodexAccountStatistics,
) {
  let state: AccountStatisticsState = { statistics: null, loading: false, refreshing: false, error: null };
  let generation = 0;
  let source: RemoteSource | null = null;
  let inFlight: Request | null = null;
  const listeners = new Set<() => void>();
  const update = (changes: Partial<AccountStatisticsState>) => {
    state = { ...state, ...changes };
    listeners.forEach((listener) => listener());
  };

  function matchesSource(statistics: AccountUsageSnapshot, selected: RemoteSource) {
    return (statistics.source === 'oauth' || statistics.source === 'pat' || statistics.source === 'cli')
      && (selected === 'auto' || statistics.source === selected);
  }

  async function validatedCache(selected: RemoteSource) {
    try {
      const cached = await readCached(selected);
      return cached?.status === 'ready' && matchesSource(cached, selected) ? cached : null;
    } catch {
      // A damaged cache must neither preserve an unverified account nor prevent
      // a successful server request from rebuilding the cache.
      return null;
    }
  }

  function start(selected: RemoteSource, initialize: boolean, mode: RefreshMode): Promise<void> {
    const request: Request = {
      source: selected, generation: ++generation, fetchRequired: !initialize,
      promise: Promise.resolve(),
    };
    inFlight = request;
    const previous = source === selected && !initialize ? state.statistics : null;
    source = selected;
    update({ statistics: previous, loading: mode === 'manual', refreshing: !initialize, error: null });
    request.promise = Promise.resolve().then(async () => {
      if (request.generation !== generation) return;
      const cached = await validatedCache(selected);
      if (request.generation !== generation) return;
      update({ statistics: cached });
      if (cached && !request.fetchRequired) {
        update({ loading: false, refreshing: false });
        return;
      }
      update({ refreshing: true });
      try {
        const statistics = await fetchStatistics(selected);
        if (request.generation !== generation) return;
        if (statistics.status !== 'ready') throw new Error(statistics.message || '服务端暂未提供使用统计，请重试。');
        if (!matchesSource(statistics, selected)) throw new Error('返回的统计来源不匹配，请重新读取。');
        update({ statistics, loading: false, refreshing: false, error: null });
      } catch (reason) {
        if (request.generation !== generation) return;
        const error = reason instanceof Error ? reason.message : typeof reason === 'string' ? reason : '读取服务端统计失败，请重试。';
        // Authentication can change while the network request is in flight.
        // Revalidate on disk instead of falling back to the previous UI state.
        const cached = await validatedCache(selected);
        if (request.generation !== generation) return;
        update({ statistics: cached, loading: false, refreshing: false, error });
      }
    }).finally(() => {
      if (inFlight === request) inFlight = null;
    });
    return request.promise;
  }

  return {
    getSnapshot: () => state,
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
    cancel: () => { generation += 1; inFlight = null; source = null; },
    initialize: (selected: RemoteSource) => start(selected, true, 'background'),
    refresh: (selected: RemoteSource, mode: RefreshMode = 'background') => {
      if (inFlight?.source === selected) {
        inFlight.fetchRequired = true;
        // An explicit click upgrades an automatic request without duplicating it.
        // Later automatic ticks must not remove the user's loading feedback.
        update({ refreshing: true, loading: state.loading || mode === 'manual' });
        return inFlight.promise;
      }
      return start(selected, false, mode);
    },
  };
}
