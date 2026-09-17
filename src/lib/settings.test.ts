import { describe, expect, it } from 'vitest';
import { codexStatisticsSources, defaultSettings, validateSettings } from './settings';

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
  it('migrates older settings to local statistics', () => {
    expect(validateSettings({ refreshIntervalSeconds: 300, enabledProviders: ['codex'], theme: 'system' }))
      .toMatchObject({ codexStatisticsSource: 'local' });
  });
  it.each(['local', 'auto'])('preserves selected statistics source %s', (codexStatisticsSource) => {
    expect(validateSettings({ ...defaultSettings(), codexStatisticsSource }))
      .toMatchObject({ codexStatisticsSource });
  });
  it.each(['oauth', 'pat', 'cli'])('migrates legacy %s selection to automatic server selection', (codexStatisticsSource) => {
    expect(validateSettings({ ...defaultSettings(), codexStatisticsSource }))
      .toMatchObject({ codexStatisticsSource: 'auto' });
  });
  it('offers only local records and the server', () => {
    expect(codexStatisticsSources).toEqual([
      { value: 'local', label: '本机记录' },
      { value: 'auto', label: '服务端' },
    ]);
  });
  it('rejects unsupported sources', () => {
    expect(() => validateSettings({ ...defaultSettings(), codexStatisticsSource: 'web' })).toThrow('来源');
  });
  it.each([true, false, 'false'])('discards removed webpage settings while preserving preferences: %s', (codexWebExtras) => {
    const preferences = { ...defaultSettings(), codexStatisticsSource: 'auto', theme: 'dark', enabledProviders: ['codex'] };
    expect(validateSettings({ ...preferences, codexWebExtras })).toEqual(preferences);
  });
});
