import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { StatisticsPanelState } from './panel';

const native = vi.hoisted(() => ({ desktop: true, invoke: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => native.desktop, invoke: native.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: native.listen }));

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => { resolve = done; });
  return { promise, resolve };
}

beforeEach(() => {
  vi.resetModules();
  vi.resetAllMocks();
  native.desktop = true;
});

describe('statistics panel intent ordering', () => {
  it('finishes a pending hover open and close before opening the next provider', async () => {
    const opening = deferred();
    const closing = deferred();
    native.invoke.mockResolvedValue(undefined)
      .mockReturnValueOnce(opening.promise)
      .mockReturnValueOnce(closing.promise);
    const panel = await import('./panel');

    const codexAnchor = { x: 12, y: 80, width: 336, height: 148 };
    const claudeAnchor = { x: 12, y: 240, width: 336, height: 160 };
    const first = panel.showStatisticsPanel('codex', codexAnchor);
    const close = panel.hideStatisticsPanel();
    const next = panel.showStatisticsPanel('claude', claudeAnchor, true);

    await vi.waitFor(() => expect(native.invoke).toHaveBeenCalledTimes(1));
    expect(native.invoke).toHaveBeenLastCalledWith('show_statistics_panel', { provider: 'codex', anchor: codexAnchor, focus: false, updateOnly: false });
    opening.resolve();
    await first;
    await vi.waitFor(() => expect(native.invoke).toHaveBeenCalledTimes(2));
    expect(native.invoke).toHaveBeenLastCalledWith('hide_statistics_panel', { focusMain: false });
    closing.resolve();
    await Promise.all([close, next]);
    expect(native.invoke.mock.calls).toEqual([
      ['show_statistics_panel', { provider: 'codex', anchor: codexAnchor, focus: false, updateOnly: false }],
      ['hide_statistics_panel', { focusMain: false }],
      ['show_statistics_panel', { provider: 'claude', anchor: claudeAnchor, focus: true, updateOnly: false }],
    ]);
  });

  it('captures the card rectangle when an open is queued instead of using later caller mutations', async () => {
    const blocking = deferred();
    native.invoke.mockResolvedValue(undefined).mockReturnValueOnce(blocking.promise);
    const panel = await import('./panel');
    const close = panel.hideStatisticsPanel();
    await vi.waitFor(() => expect(native.invoke).toHaveBeenCalledTimes(1));

    const anchor = { x: 12, y: 80, width: 336, height: 148 };
    const open = panel.showStatisticsPanel('codex', anchor);
    Object.assign(anchor, { x: 24, y: 240, width: 280, height: 180 });
    expect(native.invoke).toHaveBeenCalledTimes(1);

    blocking.resolve();
    await Promise.all([close, open]);
    expect(native.invoke).toHaveBeenLastCalledWith('show_statistics_panel', {
      provider: 'codex', anchor: { x: 12, y: 80, width: 336, height: 148 }, focus: false, updateOnly: false,
    });
  });

  it('marks late layout updates as geometry-only so native can ignore them after dismissal', async () => {
    native.invoke.mockResolvedValue(undefined);
    const panel = await import('./panel');
    await panel.hideStatisticsPanel();
    await panel.updateStatisticsPanelAnchor('codex', { x: 12, y: 20, width: 336, height: 148 });
    expect(native.invoke.mock.calls).toEqual([
      ['hide_statistics_panel', { focusMain: false }],
      ['show_statistics_panel', { provider: 'codex', anchor: { x: 12, y: 20, width: 336, height: 148 }, focus: false, updateOnly: true }],
    ]);
  });

  it('reports a failed open without preventing later interaction, dismissal, and reopening', async () => {
    native.invoke.mockResolvedValue(undefined).mockRejectedValueOnce(new Error('window unavailable'));
    const panel = await import('./panel');
    const failure = expect(panel.showStatisticsPanel('codex', { x: 12, y: 80, width: 336, height: 148 })).rejects.toThrow('window unavailable');
    const interaction = panel.setPanelInteraction(false, true, 'pointer');
    const dismissal = panel.dismissPanel();
    const reopen = panel.showStatisticsPanel('claude', { x: 12, y: 240, width: 336, height: 160 });

    await Promise.all([failure, interaction, dismissal, reopen]);
    expect(native.invoke).toHaveBeenCalledWith('set_panel_interaction', { hovered: false, keyboard: true, intent: 'pointer' });
    expect(native.invoke.mock.calls.map(([command]) => command)).toEqual([
      'show_statistics_panel', 'set_panel_interaction', 'dismiss_panel', 'show_statistics_panel',
    ]);
  });
});

describe('statistics panel state boundary', () => {
  it('delivers native revisions unchanged and releases the event listener', async () => {
    const stop = vi.fn();
    native.listen.mockResolvedValue(stop);
    const initial: StatisticsPanelState = { provider: 'codex', side: 'left', revision: 12 };
    native.invoke.mockResolvedValueOnce(initial).mockResolvedValue(undefined);
    const panel = await import('./panel');
    expect(await panel.getStatisticsPanelState()).toEqual(initial);

    const receive = vi.fn();
    const unsubscribe = await panel.subscribeToStatisticsPanel(receive);
    expect(native.listen).toHaveBeenCalledWith('statistics-panel-changed', expect.any(Function));
    const changed = native.listen.mock.calls[0][1] as (event: { payload: StatisticsPanelState }) => void;
    const closed: StatisticsPanelState = { provider: null, side: null, revision: 13 };
    changed({ payload: initial });
    changed({ payload: closed });
    expect(receive.mock.calls).toEqual([[initial], [closed]]);

    // A render can complete after dismissal. Preserve its old revision so native
    // code can reject that acknowledgement instead of reopening the window.
    await panel.presentStatisticsPanel(initial.revision);
    expect(native.invoke).toHaveBeenLastCalledWith('present_statistics_panel', { revision: 12 });
    unsubscribe();
    expect(stop).toHaveBeenCalledOnce();
  });

  it('keeps browser previews closed without invoking or subscribing to native APIs', async () => {
    native.desktop = false;
    const panel = await import('./panel');
    const receive = vi.fn();
    const unsubscribe = await panel.subscribeToStatisticsPanel(receive);
    await Promise.all([
      panel.showStatisticsPanel('codex', { x: 12, y: 80, width: 336, height: 148 }, true), panel.hideStatisticsPanel(true),
      panel.dismissPanel(), panel.setPanelInteraction(true, true), panel.presentStatisticsPanel(4),
    ]);
    expect(await panel.getStatisticsPanelState()).toEqual({ provider: null, side: null, revision: 0 });
    unsubscribe();
    expect(native.invoke).not.toHaveBeenCalled();
    expect(native.listen).not.toHaveBeenCalled();
    expect(receive).not.toHaveBeenCalled();
  });
});
