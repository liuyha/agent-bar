// Run with: node --test src-tauri/src/codex_web/read_usage.check.mjs
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { setImmediate } from 'node:timers/promises';
import test from 'node:test';
import vm from 'node:vm';

const source = readFileSync(new URL('./read_usage.js', import.meta.url), 'utf8');
const token = 'fixture-session-token-never-exported';
const session = { accessToken: token, user: { email: 'User@Example.test' }, private: 'private-session-data' };
const endpoints = {
  '/api/auth/session': session,
  '/backend-api/wham/usage': {
    account_id: 'confirmed-workspace', credits: { balance: '42.5' },
  },
  '/backend-api/wham/usage/daily-token-usage-breakdown': {
    units: 'credits', data: [{ date: '2026-09-17', product_surface_usage_values: { codex_cli: 5.25 } }],
  },
  '/backend-api/wham/usage/credit-usage-events': {
    data: [{ date: '2026-09-17T03:00:00Z', product_surface: 'codex_cli', credit_amount: -1.5 }],
  },
};

function start({ config = {}, overrides = {}, origin = 'https://chatgpt.com', fetcher, fastEndpointTimeout = false,
  bodyText = 'Code review\n87.5% remaining', pathname = '/codex/settings/usage' } = {}) {
  const calls = [];
  const window = {};
  const location = { origin, pathname };
  const context = vm.createContext({
    window, location, document: { body: { innerText: bodyText } }, AbortController, URL,
    setTimeout: (callback, ms) => setTimeout(callback, fastEndpointTimeout && ms === 7000 ? 5 : ms),
    clearTimeout,
    fetch: async (url, options) => {
      calls.push({ url, options });
      if (fetcher) return fetcher(url, options);
      const path = new URL(url).pathname;
      const result = Object.hasOwn(overrides, path) ? overrides[path] : endpoints[path];
      if (result instanceof Error) throw result;
      return { ok: result !== false, json: async () => result };
    },
  });
  const settings = { requestId: 123, expectedAccount: ' user@example.test ', expectedAccountId: 'confirmed-workspace', ...config };
  vm.runInContext(source.replace('/*__AGENTBAR_WEB_CONFIG_JSON__*/null', JSON.stringify(settings)), context);
  return { calls, window, location, context };
}

async function read(options) {
  const run = start(options);
  for (let i = 0; i < 20 && !run.window.__agentbarWebUsageV1?.done; i++) await setImmediate();
  assert.equal(run.window.__agentbarWebUsageV1?.done, true, 'fixture must finish without real network');
  return { ...run, snapshot: JSON.parse(JSON.stringify(run.window.__agentbarWebUsageV1.snapshot)) };
}

test('same-account session uses backend-confirmed workspace and keeps credit units separate', async () => {
  const { snapshot, calls } = await read();
  assert.equal(snapshot.status, 'ready');
  assert.equal(snapshot.account, 'user@example.test');
  assert.equal(snapshot.creditsRemaining, 42.5);
  assert.equal(snapshot.codeReviewRemainingPercent, 87.5);
  assert.equal(snapshot.usageUnit, 'credits');
  assert.deepEqual(snapshot.usageBreakdown, [{ date: '2026-09-17', amounts: [{ service: 'codex_cli', amount: 5.25 }] }]);
  assert.equal(snapshot.creditEvents[0].credits, -1.5);
  assert.equal(snapshot.message, null);
  assert.ok(snapshot.updatedAt);
  assert.equal(calls.length, 5);
  for (const { url, options } of calls) {
    assert.equal(new URL(url).origin, 'https://chatgpt.com');
    assert.equal(options.redirect, 'error');
    assert.equal(options.credentials, 'same-origin');
    if (new URL(url).pathname !== '/api/auth/session') {
      assert.equal(options.headers.Authorization, `Bearer ${token}`);
      if (options.headers['ChatGPT-Account-Id'] !== undefined) {
        assert.equal(options.headers['ChatGPT-Account-Id'], 'confirmed-workspace');
      } else assert.equal(new URL(url).pathname, '/backend-api/wham/usage');
    } else assert.equal(options.headers.Authorization, undefined);
  }
  const daily = new URL(calls.find(({ url }) => url.includes('daily-token')).url);
  assert.equal(daily.searchParams.get('group_by'), 'day');
  const days = (new Date(daily.searchParams.get('end_date')) - new Date(daily.searchParams.get('start_date'))) / 86400000;
  assert.equal(days, 29, 'UTC inclusive range is 30 days');
  assert.ok(!JSON.stringify(snapshot).includes(token));
  assert.ok(!JSON.stringify(snapshot).includes('private-session-data'));
});

test('mismatched browser identity blocks every usage request and exports no browser identity', async () => {
  const { snapshot, calls } = await read({ overrides: { '/api/auth/session': { ...session, user: { email: 'someone-else@example.test' } } } });
  assert.equal(snapshot.status, 'unavailable');
  assert.equal(snapshot.account, null);
  assert.equal(snapshot.creditsRemaining, null);
  assert.equal(calls.length, 1);
  assert.ok(snapshot.message.includes('不一致'));
});

test('missing backend workspace identity never queries the browser session', async () => {
  const { snapshot, calls } = await read({ config: { expectedAccountId: null } });
  assert.equal(snapshot.status, 'unavailable');
  assert.equal(calls.length, 0);
});

test('one forbidden endpoint retains independent successful results with fixed partial explanation', async () => {
  const { snapshot } = await read({ overrides: { '/backend-api/wham/usage/daily-token-usage-breakdown': false } });
  assert.equal(snapshot.status, 'ready');
  assert.equal(snapshot.usageBreakdown, null);
  assert.equal(snapshot.creditsRemaining, 42.5);
  assert.equal(snapshot.creditEvents[0].credits, -1.5);
  assert.equal(snapshot.message, '部分网页补充数据暂不可用：每日用量。');
});

test('a hung endpoint times out independently and preserves the two successful endpoints', async () => {
  const run = start({
    fastEndpointTimeout: true,
    fetcher: async (url) => {
      const path = new URL(url).pathname;
      if (path.endsWith('credit-usage-events')) return new Promise(() => {});
      return { ok: true, json: async () => endpoints[path] };
    },
  });
  await new Promise((resolve) => setTimeout(resolve, 15));
  const snapshot = run.window.__agentbarWebUsageV1.snapshot;
  assert.equal(snapshot.status, 'ready');
  assert.equal(snapshot.creditsRemaining, 42.5);
  assert.equal(snapshot.usageBreakdown[0].amounts[0].amount, 5.25);
  assert.equal(snapshot.creditEvents, null);
  assert.equal(snapshot.message, '部分网页补充数据暂不可用：Credit 记录。');
});

test('absent units remain unknown and successful empty datasets remain empty', async () => {
  const { snapshot } = await read({ overrides: {
    '/backend-api/wham/usage/daily-token-usage-breakdown': { data: [] },
    '/backend-api/wham/usage/credit-usage-events': { data: [] },
  } });
  assert.equal(snapshot.usageUnit, null);
  assert.deepEqual(snapshot.usageBreakdown, []);
  assert.deepEqual(snapshot.creditEvents, []);
  assert.equal(snapshot.status, 'ready');
});

test('zero balances and fully used code-review quotas are actual zeros', async () => {
  const { snapshot } = await read({ bodyText: 'Code review\n100% used', overrides: {
    '/backend-api/wham/usage': { account_id: 'confirmed-workspace', credits: { balance: 0 } },
  } });
  assert.equal(snapshot.creditsRemaining, 0);
  assert.equal(snapshot.codeReviewRemainingPercent, 0);
});

test('malformed amounts do not silently become zero or erase another endpoint', async () => {
  const { snapshot } = await read({ overrides: {
    '/backend-api/wham/usage': { credits: { balance: '' } },
    '/backend-api/wham/usage/daily-token-usage-breakdown': { data: [{ date: '2026-09-17', product_surface_usage_values: { codex: null } }] },
  } });
  assert.equal(snapshot.status, 'ready');
  assert.equal(snapshot.creditsRemaining, null);
  assert.equal(snapshot.codeReviewRemainingPercent, null);
  assert.equal(snapshot.usageBreakdown, null);
  assert.equal(snapshot.creditEvents[0].credits, -1.5);
});

test('Code review DOM extraction supports English and Chinese remaining/used labels', async () => {
  for (const [bodyText, expected] of [
    ['Code review\n80% remaining', 80], ['Core review\n25% left', 25],
    ['Code review: 25% used', 75], ['Code review remaining: 66.5%', 66.5],
    ['代码审查\n剩余 80%', 80], ['代码审阅额度\n已使用 12.5%', 87.5],
    ['代码评审 20% 已用', 80],
  ]) {
    const { snapshot } = await read({ bodyText });
    assert.equal(snapshot.codeReviewRemainingPercent, expected, bodyText);
  }
});

test('Code review DOM rejects unrelated, ambiguous, out-of-range and wrong-page text', async () => {
  for (const options of [
    { bodyText: 'Weekly limit 75% remaining' },
    { bodyText: 'Code review\nWeekly limit 75% remaining' },
    { bodyText: 'Code review 75% remaining\nCode review 30% remaining' },
    { bodyText: 'Code review 120% remaining' },
    { bodyText: 'Code review 75% remaining', pathname: '/c/example' },
  ]) {
    const { snapshot } = await read(options);
    assert.equal(snapshot.codeReviewRemainingPercent, null);
    assert.equal(snapshot.creditsRemaining, 42.5);
  }
});

test('Code review requires confirmed default webpage workspace, even when email matches', async () => {
  for (const account of ['other-workspace', undefined]) {
    const { snapshot } = await read({ fetcher: async (url, options) => {
      const path = new URL(url).pathname;
      const value = path.endsWith('/usage') && !options.headers['ChatGPT-Account-Id']
        ? { account_id: account } : endpoints[path];
      return { ok: true, json: async () => value };
    } });
    assert.equal(snapshot.codeReviewRemainingPercent, null);
    assert.equal(snapshot.creditsRemaining, 42.5);
    assert.equal(snapshot.status, 'ready');
  }
});

test('default webpage workspace timeout does not suppress scoped API results', async () => {
  const run = start({ fastEndpointTimeout: true, fetcher: async (url, options) => {
    const path = new URL(url).pathname;
    if (path.endsWith('/usage') && !options.headers['ChatGPT-Account-Id']) return new Promise(() => {});
    return { ok: true, json: async () => endpoints[path] };
  } });
  await new Promise((resolve) => setTimeout(resolve, 15));
  const snapshot = run.window.__agentbarWebUsageV1.snapshot;
  assert.equal(snapshot.codeReviewRemainingPercent, null);
  assert.equal(snapshot.creditsRemaining, 42.5);
  assert.equal(snapshot.creditEvents[0].credits, -1.5);
});

test('API workspace mismatch discards all values even when other endpoints succeeded', async () => {
  const { snapshot } = await read({ overrides: { '/backend-api/wham/usage': { ...endpoints['/backend-api/wham/usage'], account_id: 'another-workspace' } } });
  assert.equal(snapshot.status, 'unavailable');
  assert.equal(snapshot.account, null);
  assert.equal(snapshot.usageBreakdown, null);
  assert.equal(snapshot.creditEvents, null);
});

test('total endpoint failure exports only a fixed message, never HTTP bodies or exception text', async () => {
  const { snapshot } = await read({ overrides: {
    '/backend-api/wham/usage': new Error(`private HTTP error ${token}`),
    '/backend-api/wham/usage/daily-token-usage-breakdown': false,
    '/backend-api/wham/usage/credit-usage-events': false,
  } });
  assert.equal(snapshot.status, 'error');
  assert.equal(snapshot.updatedAt, null);
  assert.ok(!JSON.stringify(snapshot).includes(token));
});

test('login or external pages cannot run extraction', () => {
  const run = start({ origin: 'https://auth.openai.com' });
  assert.equal(run.calls.length, 0);
  assert.equal(run.window.__agentbarWebUsageV1, undefined);
});

test('cancellation while a session request is pending cannot publish a late result', async () => {
  let resolve;
  const run = start({ fetcher: () => new Promise((done) => { resolve = done; }) });
  run.window.__agentbarWebUsageV1.cancel();
  resolve({ ok: true, json: async () => ({ ...session, user: { email: 'wrong@example.test' } }) });
  for (let i = 0; i < 4; i++) await setImmediate();
  assert.equal(run.window.__agentbarWebUsageV1.done, false);
  assert.equal(run.window.__agentbarWebUsageV1.snapshot, null);
});

test('navigation during a pending read cannot publish data to a login page', async () => {
  let resolve;
  const run = start({ fetcher: () => new Promise((done) => { resolve = done; }) });
  run.location.origin = 'https://auth.openai.com';
  resolve({ ok: true, json: async () => ({ ...session, user: { email: 'wrong@example.test' } }) });
  for (let i = 0; i < 4; i++) await setImmediate();
  assert.equal(run.window.__agentbarWebUsageV1.done, false);
  assert.equal(run.window.__agentbarWebUsageV1.snapshot, null);
});
