import { describe, expect, it, vi } from 'vitest';
import type { AccountUsageSnapshot } from '../types';
import { createAccountStatisticsStore, formatDuration } from './accountStatistics';

function result(source: AccountUsageSnapshot['source'] = 'oauth', account = 'first'): AccountUsageSnapshot {
  return { source, status: 'ready', message: null, account, accountId: null, summary: { lifetimeTokens: 123, peakDailyTokens: null, longestRunningTurnSec: null, currentStreakDays: null, longestStreakDays: null }, dailyUsage: null, serviceUpdatedAt: null, updatedAt: null };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((accept, fail) => { resolve = accept; reject = fail; });
  return { promise, resolve, reject };
}

const emptyCache = async () => null;

async function nextStage() {
  // Start the promise pipeline and pass the cache-read await, without resolving
  // whichever server or validation response the test deliberately holds open.
  for (let count = 0; count < 6; count += 1) await Promise.resolve();
}

describe('remote statistics cache and refresh modes', () => {
  it('starts without panel loading and restores the validated cache without fetching', async () => {
    const cached = result();
    const fetch = vi.fn();
    const read = vi.fn().mockResolvedValue(cached);
    const store = createAccountStatisticsStore(fetch, read);
    expect(store.getSnapshot()).toEqual({ statistics: null, loading: false, refreshing: false, error: null });
    await store.initialize('oauth');
    expect(read).toHaveBeenCalledWith('oauth');
    expect(fetch).not.toHaveBeenCalled();
    expect(store.getSnapshot()).toEqual({ statistics: cached, loading: false, refreshing: false, error: null });
  });

  it('fetches a cache miss silently and exposes only refresh-button progress', async () => {
    const response = deferred<AccountUsageSnapshot>();
    const fetch = vi.fn(() => response.promise);
    const store = createAccountStatisticsStore(fetch, emptyCache);
    const pending = store.initialize('oauth');
    await nextStage();
    expect(fetch).toHaveBeenCalledWith('oauth');
    expect(store.getSnapshot()).toEqual({ statistics: null, loading: false, refreshing: true, error: null });
    response.resolve(result());
    await pending;
    expect(store.getSnapshot()).toMatchObject({ statistics: result(), loading: false, refreshing: false });
  });

  it.each(['background', 'manual'] as const)('keeps cached values during a %s refresh', async (mode) => {
    const cached = result();
    const response = deferred<AccountUsageSnapshot>();
    const store = createAccountStatisticsStore(() => response.promise, async () => cached);
    await store.initialize('oauth');
    const pending = store.refresh('oauth', mode);
    expect(store.getSnapshot()).toEqual({ statistics: cached, loading: mode === 'manual', refreshing: true, error: null });
    await nextStage();
    expect(store.getSnapshot()).toEqual({ statistics: cached, loading: mode === 'manual', refreshing: true, error: null });
    response.resolve(result('oauth', 'updated'));
    await pending;
    expect(store.getSnapshot()).toMatchObject({ statistics: { account: 'updated' }, loading: false, refreshing: false });
  });

  it('merges concurrent refreshes, upgrades manual feedback, and never downgrades it on automatic ticks', async () => {
    const response = deferred<AccountUsageSnapshot>();
    const fetch = vi.fn(() => response.promise);
    const store = createAccountStatisticsStore(fetch, async () => result());
    await store.initialize('oauth');
    const background = store.refresh('oauth');
    await nextStage();
    const manual = store.refresh('oauth', 'manual');
    const tick = store.refresh('oauth', 'background');
    expect(manual).toBe(background);
    expect(tick).toBe(background);
    expect(fetch).toHaveBeenCalledOnce();
    expect(store.getSnapshot()).toMatchObject({ loading: true, refreshing: true });
    response.resolve(result());
    await manual;
    expect(store.getSnapshot()).toMatchObject({ loading: false, refreshing: false });
  });

  it('honors a manual click while startup is still reading its cache', async () => {
    const cache = deferred<AccountUsageSnapshot | null>();
    const response = deferred<AccountUsageSnapshot>();
    const fetch = vi.fn(() => response.promise);
    const store = createAccountStatisticsStore(fetch, () => cache.promise);
    const initial = store.initialize('oauth');
    const manual = store.refresh('oauth', 'manual');
    expect(initial).toBe(manual);
    cache.resolve(result());
    await nextStage();
    expect(fetch).toHaveBeenCalledOnce();
    expect(store.getSnapshot()).toMatchObject({ statistics: result(), loading: true, refreshing: true });
    response.resolve(result());
    await manual;
  });

  it.each(['throw', 'error', 'unavailable'] as const)('retains only a revalidated successful cache after a %s response', async (kind) => {
    const cached = result();
    const read = vi.fn().mockResolvedValue(cached);
    const fetch = vi.fn(async () => {
      if (kind === 'throw') throw new Error('服务暂时不可用');
      return { ...result(), status: kind, message: '服务暂时不可用' };
    });
    const store = createAccountStatisticsStore(fetch, read);
    await store.initialize('oauth');
    await store.refresh('oauth');
    expect(read).toHaveBeenCalledTimes(3);
    expect(store.getSnapshot()).toEqual({ statistics: cached, loading: false, refreshing: false, error: '服务暂时不可用' });
  });

  it('clears stale account data before fetching when validation no longer finds its cache', async () => {
    const response = deferred<AccountUsageSnapshot>();
    const read = vi.fn().mockResolvedValueOnce(result()).mockResolvedValue(null);
    const store = createAccountStatisticsStore(() => response.promise, read);
    await store.initialize('oauth');
    const pending = store.refresh('oauth');
    await nextStage();
    expect(store.getSnapshot()).toEqual({ statistics: null, loading: false, refreshing: true, error: null });
    response.reject(new Error('账号认证失败'));
    await pending;
    expect(store.getSnapshot()).toEqual({ statistics: null, loading: false, refreshing: false, error: '账号认证失败' });
  });

  it('drops a former account cache if authentication changes during a failed request', async () => {
    const response = deferred<AccountUsageSnapshot>();
    const read = vi.fn().mockResolvedValueOnce(result()).mockResolvedValueOnce(result()).mockResolvedValue(null);
    const store = createAccountStatisticsStore(() => response.promise, read);
    await store.initialize('oauth');
    const pending = store.refresh('oauth', 'manual');
    await nextStage();
    response.reject(new Error('账号已切换'));
    await pending;
    expect(store.getSnapshot()).toEqual({ statistics: null, loading: false, refreshing: false, error: '账号已切换' });
  });

  it('recovers from corrupt cache reads by fetching fresh statistics', async () => {
    const read = vi.fn().mockRejectedValue(new Error('缓存损坏'));
    const fetch = vi.fn().mockResolvedValue(result());
    const store = createAccountStatisticsStore(fetch, read);
    await store.initialize('oauth');
    expect(fetch).toHaveBeenCalledOnce();
    expect(store.getSnapshot()).toEqual({ statistics: result(), loading: false, refreshing: false, error: null });
  });

  it('never falls back to UI data when cache revalidation fails', async () => {
    const read = vi.fn().mockResolvedValueOnce(result()).mockResolvedValueOnce(result()).mockRejectedValue(new Error('配置不可用'));
    const store = createAccountStatisticsStore(async () => { throw new Error('认证失败'); }, read);
    await store.initialize('oauth');
    await store.refresh('oauth');
    expect(store.getSnapshot()).toEqual({ statistics: null, loading: false, refreshing: false, error: '认证失败' });
  });

  it.each(['error', 'unavailable'] as const)('never restores a cached %s snapshot as successful data', async (status) => {
    const fetch = vi.fn().mockResolvedValue(result());
    const store = createAccountStatisticsStore(fetch, async () => ({ ...result(), status }));
    await store.initialize('oauth');
    expect(fetch).toHaveBeenCalledOnce();
    expect(store.getSnapshot().statistics?.status).toBe('ready');
  });
});

describe('remote statistics source and account isolation', () => {
  it('discards an older source response after a newer request completes', async () => {
    const oauth = deferred<AccountUsageSnapshot>();
    const pat = deferred<AccountUsageSnapshot>();
    const store = createAccountStatisticsStore(vi.fn((source) => source === 'oauth' ? oauth.promise : pat.promise), emptyCache);
    const first = store.refresh('oauth');
    await nextStage();
    const second = store.refresh('pat');
    expect(store.getSnapshot().statistics).toBeNull();
    pat.resolve(result('pat', 'second'));
    await second;
    oauth.resolve(result('oauth', 'first'));
    await first;
    expect(store.getSnapshot().statistics).toMatchObject({ source: 'pat', account: 'second' });
  });

  it('does not start a stale fetch after switching sources during cache restoration', async () => {
    const oldCache = deferred<AccountUsageSnapshot | null>();
    const read = vi.fn((source) => source === 'oauth' ? oldCache.promise : Promise.resolve(result('pat', 'second')));
    const fetch = vi.fn();
    const store = createAccountStatisticsStore(fetch, read);
    const first = store.initialize('oauth');
    await nextStage();
    await store.initialize('pat');
    oldCache.resolve(null);
    await first;
    expect(fetch).not.toHaveBeenCalled();
    expect(store.getSnapshot().statistics?.account).toBe('second');
  });

  it('rejects mismatched caches, explicit server sources, and local data in automatic mode', async () => {
    const fetch = vi.fn().mockResolvedValue(result('pat'));
    const store = createAccountStatisticsStore(fetch, async () => result('pat'));
    await store.initialize('oauth');
    expect(fetch).toHaveBeenCalledWith('oauth');
    expect(store.getSnapshot().statistics).toBeNull();
    expect(store.getSnapshot().error).toContain('来源不匹配');
    await store.initialize('auto');
    expect(store.getSnapshot().statistics?.source).toBe('pat');
    const local = createAccountStatisticsStore(async () => result('local'), async () => result('local'));
    await local.initialize('auto');
    expect(local.getSnapshot().statistics).toBeNull();
    expect(local.getSnapshot().error).toContain('来源不匹配');
  });

  it.each(['auto', 'cli'] as const)('accepts a successful CLI response selected through %s', async (source) => {
    const fetch = vi.fn().mockResolvedValue(result('cli'));
    const store = createAccountStatisticsStore(fetch, emptyCache);
    await store.initialize(source);
    expect(fetch).toHaveBeenCalledWith(source);
    expect(store.getSnapshot()).toEqual({ statistics: result('cli'), loading: false, refreshing: false, error: null });
  });

  it('preserves an automatic-source unavailable message before validating its concrete source', async () => {
    const store = createAccountStatisticsStore(async () => ({ ...result('auto'), status: 'unavailable', message: '服务端使用统计仅在桌面应用中可用。' }), emptyCache);
    await store.initialize('auto');
    expect(store.getSnapshot().error).toBe('服务端使用统计仅在桌面应用中可用。');
  });

  it('keeps the original connection error and validated cache when a failed automatic request has no concrete source', async () => {
    const cached = result();
    const store = createAccountStatisticsStore(async () => ({ ...result('auto'), status: 'error', message: '服务端连接失败' }), async () => cached);
    await store.initialize('auto');
    await store.refresh('auto');
    expect(store.getSnapshot()).toEqual({ statistics: cached, loading: false, refreshing: false, error: '服务端连接失败' });
  });

  it('ignores cancelled responses and restores the disk cache for a new mounted view', async () => {
    const response = deferred<AccountUsageSnapshot>();
    const original = createAccountStatisticsStore(() => response.promise, emptyCache);
    const pending = original.refresh('oauth');
    await nextStage();
    original.cancel();
    const cancelled = original.getSnapshot();
    const fetch = vi.fn();
    const next = createAccountStatisticsStore(fetch, async () => result('oauth', 'second'));
    expect(next.getSnapshot().statistics).toBeNull();
    await next.initialize('oauth');
    response.resolve(result());
    await pending;
    expect(original.getSnapshot()).toBe(cancelled);
    expect(fetch).not.toHaveBeenCalled();
    expect(next.getSnapshot().statistics?.account).toBe('second');
  });

  it('can initialize the same store again after effect cleanup without publishing its cancelled cache read', async () => {
    const oldCache = deferred<AccountUsageSnapshot | null>();
    const read = vi.fn().mockReturnValueOnce(oldCache.promise).mockResolvedValueOnce(result('oauth', 'second'));
    const fetch = vi.fn();
    const store = createAccountStatisticsStore(fetch, read);
    const cancelled = store.initialize('oauth');
    await nextStage();
    store.cancel();
    await store.initialize('oauth');
    oldCache.resolve(result());
    await cancelled;
    expect(fetch).not.toHaveBeenCalled();
    expect(store.getSnapshot()).toEqual({ statistics: result('oauth', 'second'), loading: false, refreshing: false, error: null });
  });

  it('does not start superseded work when setup and cleanup run synchronously', async () => {
    const read = vi.fn().mockResolvedValue(result());
    const fetch = vi.fn();
    const store = createAccountStatisticsStore(fetch, read);
    const first = store.initialize('oauth');
    store.cancel();
    const second = store.initialize('oauth');
    await Promise.all([first, second]);
    expect(read).toHaveBeenCalledOnce();
    expect(fetch).not.toHaveBeenCalled();
    expect(store.getSnapshot().statistics).toEqual(result());
  });

  it('discards recovery-cache results when a newer initialization has already completed', async () => {
    const recovery = deferred<AccountUsageSnapshot | null>();
    const read = vi.fn().mockResolvedValueOnce(null).mockReturnValueOnce(recovery.promise).mockResolvedValueOnce(result('pat', 'second'));
    const store = createAccountStatisticsStore(async () => { throw new Error('断网'); }, read);
    const pending = store.refresh('oauth');
    await nextStage();
    await store.initialize('pat');
    recovery.resolve(result());
    await pending;
    expect(store.getSnapshot()).toEqual({ statistics: result('pat', 'second'), loading: false, refreshing: false, error: null });
  });

  it.each([[null, '—'], [0, '0 秒'], [45, '45 秒'], [125, '2 分 5 秒'], [3661, '1 小时 1 分']] as const)('formats server task duration %s', (seconds, expected) => {
    expect(formatDuration(seconds)).toBe(expected);
  });
});
