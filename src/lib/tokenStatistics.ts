import type { ProviderId, TokenStatistics } from '../types';
import { getCachedTokenStatistics, getTokenStatistics } from './api';

interface StatisticsState {
  statistics: TokenStatistics | null;
  loading: boolean;
  error: string | null;
}

interface StatisticsEntry {
  state: StatisticsState;
  listeners: Set<() => void>;
  inFlight: Promise<void> | null;
}

// A panel can unmount while collection is running. Keep its latest result and request
// outside React so reopening it can immediately render the same provider's data.
const entries = new Map<ProviderId, StatisticsEntry>();

function entryFor(provider: ProviderId): StatisticsEntry {
  let entry = entries.get(provider);
  if (!entry) {
    entry = { state: { statistics: null, loading: true, error: null }, listeners: new Set(), inFlight: null };
    entries.set(provider, entry);
  }
  return entry;
}

function update(entry: StatisticsEntry, changes: Partial<StatisticsState>) {
  entry.state = { ...entry.state, ...changes };
  entry.listeners.forEach((listener) => listener());
}

export function getStatisticsState(provider: ProviderId): StatisticsState {
  return entryFor(provider).state;
}

export function subscribeToStatistics(provider: ProviderId, listener: () => void): () => void {
  const entry = entryFor(provider);
  entry.listeners.add(listener);
  return () => { entry.listeners.delete(listener); };
}

export function refreshStatistics(provider: ProviderId): Promise<void> {
  const entry = entryFor(provider);
  if (entry.inFlight) return entry.inFlight;

  entry.inFlight = Promise.resolve().then(async () => {
    if (!entry.state.statistics) {
      // Restore only the saved summary; scanning logs happens separately below.
      // A damaged/missing cache can be rebuilt by a successful fresh collection.
      try {
        const cached = await getCachedTokenStatistics(provider);
        if (cached && cached.status !== 'error') update(entry, { statistics: cached });
      } catch { /* Continue with collection and report its error if it also fails. */ }
    }
    try {
      const statistics = await getTokenStatistics(provider);
      if (statistics.status === 'error') throw new Error(statistics.message || '读取会话记录失败，请重试。');
      update(entry, { statistics, error: null });
    } catch (reason) {
      const error = reason instanceof Error ? reason.message : typeof reason === 'string' ? reason : '读取会话记录失败，请重试。';
      update(entry, { error });
    } finally {
      update(entry, { loading: false });
    }
  }).finally(() => { entry.inFlight = null; });
  update(entry, { loading: true, error: null });
  return entry.inFlight;
}
