import type { DashboardSnapshot } from '../types';

export function mergeDashboardSnapshot(
  current: DashboardSnapshot | null,
  incoming: DashboardSnapshot,
): DashboardSnapshot {
  return current && incoming.revision < current.revision ? current : incoming;
}
