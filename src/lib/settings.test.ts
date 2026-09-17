import { describe, expect, it } from 'vitest';
import { defaultSettings, validateSettings } from './settings';

describe('settings contract', () => {
  it('allows an empty selection and de-duplicates providers', () => {
    expect(validateSettings({ ...defaultSettings(), enabledProviders: [] }).enabledProviders).toEqual([]);
    expect(validateSettings({ ...defaultSettings(), enabledProviders: ['codex', 'codex'] }).enabledProviders).toEqual(['codex']);
  });
  it.each([0, -1, 61, '300', null])('rejects invalid interval %s', (refreshIntervalSeconds) => {
    expect(() => validateSettings({ ...defaultSettings(), refreshIntervalSeconds })).toThrow();
  });
  it('rejects unknown providers and themes', () => {
    expect(() => validateSettings({ ...defaultSettings(), enabledProviders: ['unknown'] })).toThrow();
    expect(() => validateSettings({ ...defaultSettings(), theme: 'unknown' })).toThrow();
  });
});
