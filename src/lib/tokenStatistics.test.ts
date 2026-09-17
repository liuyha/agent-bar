import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { TokenStatistics } from '../types';

const api = vi.hoisted(() => ({
  getCachedTokenStatistics: vi.fn(),
  getTokenStatistics: vi.fn(),
}));

vi.mock('./api', () => api);

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((accept, fail) => {
    resolve = accept;
    reject = fail;
  });
  return { promise, resolve, reject };
}

function statistics(updatedAt: string): TokenStatistics {
  return { status: 'ready', message: null, periods: [], updatedAt };
}

beforeEach(() => {
  vi.resetModules();
  vi.resetAllMocks();
  api.getCachedTokenStatistics.mockResolvedValue(null);
});

describe('statistics cache across panel openings', () => {
  it('keeps an unchanged snapshot stable before the first statistics load', async () => {
    const store = await import('./tokenStatistics');
    const state = store.getStatisticsState('codex');
    expect(state).toEqual({ statistics: null, loading: true, error: null });
    expect(store.getStatisticsState('codex')).toBe(state);
    expect(api.getCachedTokenStatistics).not.toHaveBeenCalled();
    expect(api.getTokenStatistics).not.toHaveBeenCalled();
  });

  it('shows the saved statistics before fresh collection finishes on first opening', async () => {
    const saved = statistics('2026-09-16T00:00:00Z');
    const fresh = statistics('2026-09-17T00:00:00Z');
    const disk = deferred<TokenStatistics | null>();
    const collection = deferred<TokenStatistics>();
    api.getCachedTokenStatistics.mockReturnValue(disk.promise);
    api.getTokenStatistics.mockReturnValue(collection.promise);
    const store = await import('./tokenStatistics');

    const refresh = store.refreshStatistics('codex');
    await vi.waitFor(() => expect(api.getCachedTokenStatistics).toHaveBeenCalledWith('codex'));
    expect(api.getTokenStatistics).not.toHaveBeenCalled();
    disk.resolve(saved);
    await vi.waitFor(() => {
      expect(store.getStatisticsState('codex').statistics).toEqual(saved);
      expect(api.getTokenStatistics).toHaveBeenCalledWith('codex');
    });
    collection.resolve(fresh);
    await refresh;
    expect(store.getStatisticsState('codex')).toEqual({ statistics: fresh, loading: false, error: null });
  });

  it('keeps the last result visible on reopening and replaces it after refresh', async () => {
    const first = statistics('2026-09-16T00:00:00Z');
    const next = statistics('2026-09-17T00:00:00Z');
    const collection = deferred<TokenStatistics>();
    api.getTokenStatistics.mockResolvedValueOnce(first).mockReturnValueOnce(collection.promise);
    const store = await import('./tokenStatistics');
    const stopFirstPanel = store.subscribeToStatistics('codex', vi.fn());
    await store.refreshStatistics('codex');
    stopFirstPanel();

    const stopNextPanel = store.subscribeToStatistics('codex', vi.fn());
    expect(store.getStatisticsState('codex').statistics).toEqual(first);
    const refresh = store.refreshStatistics('codex');
    expect(store.getStatisticsState('codex').statistics).toEqual(first);
    expect(api.getCachedTokenStatistics).toHaveBeenCalledTimes(1);
    collection.resolve(next);
    await refresh;
    expect(store.getStatisticsState('codex')).toEqual({ statistics: next, loading: false, error: null });
    stopNextPanel();
  });

  it('finishes and caches collection after the panel unsubscribes', async () => {
    const fresh = statistics('2026-09-17T00:00:00Z');
    const collection = deferred<TokenStatistics>();
    api.getTokenStatistics.mockReturnValue(collection.promise);
    const store = await import('./tokenStatistics');
    const listener = vi.fn();
    const unsubscribe = store.subscribeToStatistics('codex', listener);
    const refresh = store.refreshStatistics('codex');
    await vi.waitFor(() => expect(api.getTokenStatistics).toHaveBeenCalledTimes(1));
    unsubscribe();
    listener.mockClear();
    collection.resolve(fresh);
    await refresh;

    expect(listener).not.toHaveBeenCalled();
    expect(store.getStatisticsState('codex')).toEqual({ statistics: fresh, loading: false, error: null });
    const stopReopenedPanel = store.subscribeToStatistics('codex', vi.fn());
    expect(store.getStatisticsState('codex').statistics).toEqual(fresh);
    stopReopenedPanel();
  });

  it('stores a refresh even when no panel is subscribed', async () => {
    const fresh = statistics('2026-09-17T00:00:00Z');
    api.getTokenStatistics.mockResolvedValue(fresh);
    const store = await import('./tokenStatistics');
    await store.refreshStatistics('codex');
    expect(store.getStatisticsState('codex')).toEqual({ statistics: fresh, loading: false, error: null });
  });

  it('shares one collection when the same provider is refreshed concurrently', async () => {
    const disk = deferred<TokenStatistics | null>();
    const collection = deferred<TokenStatistics>();
    api.getCachedTokenStatistics.mockReturnValue(disk.promise);
    api.getTokenStatistics.mockReturnValue(collection.promise);
    const store = await import('./tokenStatistics');
    const first = store.refreshStatistics('codex');
    const second = store.refreshStatistics('codex');
    await vi.waitFor(() => expect(api.getCachedTokenStatistics).toHaveBeenCalledTimes(1));
    disk.resolve(null);
    await vi.waitFor(() => expect(api.getTokenStatistics).toHaveBeenCalledTimes(1));
    const third = store.refreshStatistics('codex');
    expect(api.getTokenStatistics).toHaveBeenCalledTimes(1);
    const fresh = statistics('2026-09-17T00:00:00Z');
    collection.resolve(fresh);
    await Promise.all([first, second, third]);
    expect(store.getStatisticsState('codex').statistics).toEqual(fresh);
  });

  it('keeps different providers and their listeners isolated', async () => {
    const codex = deferred<TokenStatistics>();
    const claude = deferred<TokenStatistics>();
    api.getTokenStatistics.mockImplementation((provider: string) => (
      provider === 'codex' ? codex.promise : claude.promise
    ));
    const store = await import('./tokenStatistics');
    const codexListener = vi.fn();
    const stop = store.subscribeToStatistics('codex', codexListener);
    const codexRefresh = store.refreshStatistics('codex');
    const claudeRefresh = store.refreshStatistics('claude');
    await vi.waitFor(() => expect(api.getTokenStatistics).toHaveBeenCalledTimes(2));
    codexListener.mockClear();
    const claudeResult = statistics('2026-09-17T01:00:00Z');
    claude.resolve(claudeResult);
    await claudeRefresh;
    expect(store.getStatisticsState('claude').statistics).toEqual(claudeResult);
    expect(store.getStatisticsState('codex').statistics).toBeNull();
    expect(codexListener).not.toHaveBeenCalled();
    const codexResult = statistics('2026-09-17T02:00:00Z');
    codex.resolve(codexResult);
    await codexRefresh;
    expect(store.getStatisticsState('codex').statistics).toEqual(codexResult);
    expect(store.getStatisticsState('claude').statistics).toEqual(claudeResult);
    expect(codexListener).toHaveBeenCalled();
    stop();
  });

  it('continues fresh collection if reading the saved cache fails', async () => {
    const fresh = statistics('2026-09-17T00:00:00Z');
    api.getCachedTokenStatistics.mockRejectedValue(new Error('缓存读取失败'));
    api.getTokenStatistics.mockResolvedValue(fresh);
    const store = await import('./tokenStatistics');
    await store.refreshStatistics('codex');
    expect(api.getTokenStatistics).toHaveBeenCalledWith('codex');
    expect(store.getStatisticsState('codex')).toEqual({ statistics: fresh, loading: false, error: null });
  });

  it('retains the last ready result on a rejected refresh and can recover', async () => {
    const saved = statistics('2026-09-16T00:00:00Z');
    const fresh = statistics('2026-09-17T00:00:00Z');
    api.getTokenStatistics
      .mockResolvedValueOnce(saved)
      .mockRejectedValueOnce(new Error('会话扫描失败'))
      .mockResolvedValueOnce(fresh);
    const store = await import('./tokenStatistics');
    await store.refreshStatistics('codex');
    await store.refreshStatistics('codex');
    expect(store.getStatisticsState('codex')).toEqual({
      statistics: saved, loading: false, error: '会话扫描失败',
    });
    await store.refreshStatistics('codex');
    expect(store.getStatisticsState('codex')).toEqual({ statistics: fresh, loading: false, error: null });
    expect(api.getCachedTokenStatistics).toHaveBeenCalledTimes(1);
  });

  it('retains the disk cache if the first fresh collection fails', async () => {
    const saved = statistics('2026-09-16T00:00:00Z');
    api.getCachedTokenStatistics.mockResolvedValue(saved);
    api.getTokenStatistics.mockRejectedValue(new Error('读取会话失败'));
    const store = await import('./tokenStatistics');
    await store.refreshStatistics('codex');
    expect(store.getStatisticsState('codex')).toEqual({
      statistics: saved, loading: false, error: '读取会话失败',
    });
  });

  it('retains successful statistics when the backend returns an error result', async () => {
    const saved = statistics('2026-09-16T00:00:00Z');
    api.getTokenStatistics.mockResolvedValueOnce(saved).mockResolvedValueOnce({
      status: 'error', message: '统计文件写入失败', periods: [], updatedAt: '2026-09-17T00:00:00Z',
    } satisfies TokenStatistics);
    const store = await import('./tokenStatistics');
    await store.refreshStatistics('codex');
    await store.refreshStatistics('codex');
    expect(store.getStatisticsState('codex')).toEqual({
      statistics: saved, loading: false, error: '统计文件写入失败',
    });
  });

  it('accepts an unavailable response as a completed statistics result', async () => {
    const unavailable: TokenStatistics = {
      status: 'unavailable', message: '未找到本机会话', periods: [], updatedAt: '2026-09-17T00:00:00Z',
    };
    api.getTokenStatistics.mockResolvedValue(unavailable);
    const store = await import('./tokenStatistics');
    await store.refreshStatistics('codex');
    expect(store.getStatisticsState('codex')).toEqual({ statistics: unavailable, loading: false, error: null });
  });
});
