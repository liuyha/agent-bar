import { describe, expect, it } from 'vitest';
import type { DashboardSnapshot } from '../types';
import { mergeDashboardSnapshot } from './snapshot';

function snapshot(revision: number, updatedAt = '2026-09-16T10:00:01Z'): DashboardSnapshot {
  return { revision, updatedAt, mode: 'live', providers: [] };
}

describe('dashboard snapshot ordering', () => {
  it('accepts the initial snapshot', () => {
    const initial = snapshot(0);
    expect(mergeDashboardSnapshot(null, initial)).toBe(initial);
  });

  it('keeps a newer event when an older boot or refresh response arrives afterward', () => {
    const newest = snapshot(8);
    const responses = [snapshot(3), snapshot(6), snapshot(1), snapshot(7)];
    const result = responses.reduce(mergeDashboardSnapshot, newest);
    expect(result).toBe(newest);
  });

  it('accepts a higher revision when timestamps are in the same second', () => {
    const current = snapshot(8);
    const next = snapshot(9);
    expect(current.updatedAt).toBe(next.updatedAt);
    expect(mergeDashboardSnapshot(current, next)).toBe(next);
    expect(mergeDashboardSnapshot(next, current)).toBe(next);
  });

  it('accepts an equivalent revision', () => {
    const current = snapshot(8);
    const equivalent = snapshot(8);
    expect(mergeDashboardSnapshot(current, equivalent)).toBe(equivalent);
  });

  it('uses revision instead of a wall clock that may move backward', () => {
    const current = snapshot(8, '2026-09-16T10:00:02Z');
    const next = snapshot(9, '2026-09-16T10:00:01Z');
    expect(mergeDashboardSnapshot(current, next)).toBe(next);
  });

  it('keeps the highest revision across interleaved response sources', () => {
    const responses = [snapshot(1), snapshot(5), snapshot(2), snapshot(7), snapshot(6), snapshot(3)];
    const result = responses.reduce<DashboardSnapshot | null>(mergeDashboardSnapshot, null);
    expect(result).toBe(responses[3]);
  });
});
