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
  it('migrates older settings to local statistics with webpage extras off', () => {
    expect(validateSettings({ refreshIntervalSeconds: 300, enabledProviders: ['codex'], theme: 'system' }))
      .toMatchObject({ codexStatisticsSource: 'local', codexWebExtras: false });
  });
  it.each(['local', 'auto'])('preserves selected statistics source %s', (codexStatisticsSource) => {
    expect(validateSettings({ ...defaultSettings(), codexStatisticsSource, codexWebExtras: true }))
      .toMatchObject({ codexStatisticsSource, codexWebExtras: true });
  });
  it.each(['oauth', 'pat', 'cli'])('migrates legacy %s selection to automatic server selection', (codexStatisticsSource) => {
    expect(validateSettings({ ...defaultSettings(), codexStatisticsSource, codexWebExtras: true }))
      .toMatchObject({ codexStatisticsSource: 'auto', codexWebExtras: true });
  });
  it('offers only local records and the server', () => {
    expect(codexStatisticsSources).toEqual([
      { value: 'local', label: '本机记录' },
      { value: 'auto', label: '服务端' },
    ]);
  });
  it('rejects unsupported sources and non-boolean webpage options', () => {
    expect(() => validateSettings({ ...defaultSettings(), codexStatisticsSource: 'web' })).toThrow('来源');
    expect(() => validateSettings({ ...defaultSettings(), codexWebExtras: 'false' })).toThrow('开关');
  });
});
