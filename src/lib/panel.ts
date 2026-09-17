import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { isDesktop } from './api';
import type { ProviderId } from '../types';

export interface StatisticsPanelState {
  provider: ProviderId | null;
  side: 'left' | 'right' | null;
  revision: number;
}

export interface PanelAnchorRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

const closedState: StatisticsPanelState = { provider: null, side: null, revision: 0 };

// Preserve pointer/keyboard intent order, including a close that follows a hover.
// Native revisions additionally reject rendering acknowledgements after dismissal.
let panelOperation: Promise<unknown> = Promise.resolve();
function enqueue<T>(operation: () => Promise<T>): Promise<T> {
  const pending = panelOperation.catch(() => {}).then(operation);
  panelOperation = pending;
  return pending;
}

export function showStatisticsPanel(provider: ProviderId, anchor: PanelAnchorRect, focus = false): Promise<void> {
  if (!isDesktop) return Promise.resolve();
  // Queue the geometry measured at the time of this intent, not a mutable DOMRect.
  const measured = { ...anchor };
  return enqueue(() => invoke('show_statistics_panel', { provider, anchor: measured, focus, updateOnly: false }));
}

export function updateStatisticsPanelAnchor(provider: ProviderId, anchor: PanelAnchorRect): Promise<void> {
  if (!isDesktop) return Promise.resolve();
  const measured = { ...anchor };
  return enqueue(() => invoke('show_statistics_panel', { provider, anchor: measured, focus: false, updateOnly: true }));
}

export function hideStatisticsPanel(focusMain = false): Promise<void> {
  if (!isDesktop) return Promise.resolve();
  return enqueue(() => invoke('hide_statistics_panel', { focusMain }));
}

export function dismissPanel(): Promise<void> {
  if (!isDesktop) return Promise.resolve();
  return enqueue(() => invoke('dismiss_panel'));
}

export function setPanelInteraction(hovered: boolean, keyboard: boolean, intent: 'pointer' | 'leave' | 'keyboard' | 'focus' = 'focus'): Promise<void> {
  if (!isDesktop) return Promise.resolve();
  return enqueue(() => invoke('set_panel_interaction', { hovered, keyboard, intent }));
}

export async function presentStatisticsPanel(revision: number): Promise<void> {
  if (isDesktop) await invoke('present_statistics_panel', { revision });
}

export function resizeContentWindow(height: number, revision?: number): Promise<void> {
  if (!isDesktop) return Promise.resolve();
  return enqueue(() => invoke('resize_content_window', { height, revision }));
}

export async function getStatisticsPanelState(): Promise<StatisticsPanelState> {
  return isDesktop ? invoke('get_statistics_panel_state') : closedState;
}

export async function subscribeToStatisticsPanel(callback: (state: StatisticsPanelState) => void): Promise<() => void> {
  if (!isDesktop) return () => {};
  return listen<StatisticsPanelState>('statistics-panel-changed', ({ payload }) => callback(payload));
}
