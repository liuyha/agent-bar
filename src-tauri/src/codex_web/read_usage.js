/* global window, location, document */
// Runs in the isolated ChatGPT webview. Never expose session tokens, cookies, or raw responses.
((config) => {
  if (!config || location.origin !== 'https://chatgpt.com') return false;
  const previous = window.__agentbarWebUsageV1;
  if (previous) previous.cancel();
  const controller = new AbortController();
  const state = {
    id: config.requestId,
    done: false,
    snapshot: null,
    cancel: () => controller.abort(),
  };
  window.__agentbarWebUsageV1 = state;
  const empty = (status, message) => ({
    status, message, account: null, creditsRemaining: null, codeReviewRemainingPercent: null,
    usageUnit: null, usageBreakdown: null, creditEvents: null, updatedAt: null,
  });
  const finish = (snapshot) => {
    if (window.__agentbarWebUsageV1 === state && !controller.signal.aborted
        && location.origin === 'https://chatgpt.com') {
      state.snapshot = snapshot;
      state.done = true;
    }
  };
  const number = (value) => {
    if (typeof value !== 'number' && !(typeof value === 'string' && /^-?\d+(?:\.\d+)?$/.test(value))) return null;
    const n = Number(value);
    return Number.isFinite(n) ? n : null;
  };
  const name = (value) => typeof value === 'string' && /^[a-zA-Z0-9_.: -]{1,80}$/.test(value) ? value : null;
  const date = (value) => {
    if (typeof value !== 'string' || value.length > 40 || !/^\d{4}-\d{2}-\d{2}(?:T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2}))?$/.test(value)) return null;
    const parsed = new Date(value);
    const day = new Date(`${value.slice(0, 10)}T00:00:00Z`);
    return Number.isFinite(parsed.getTime()) && Number.isFinite(day.getTime())
      && day.toISOString().slice(0, 10) === value.slice(0, 10) ? value : null;
  };
  // CodexBar reads this percentage from page text; it is not a documented usage API field.
  const codeReview = () => {
    if (!/^\/codex\/settings\/usage\/?$/.test(location.pathname)) return null;
    const body = document.body?.innerText;
    if (typeof body !== 'string' || body.length > 256 * 1024) return null;
    const patterns = [
      /(?:code|core)\s*review(?:\s*limit)?[\s:：-]*([0-9]{1,3}(?:\.[0-9]+)?)\s*%\s*(remaining|left|used)\b/gi,
      /(?:code|core)\s*review(?:\s*limit)?[\s:：-]*(remaining|left|used)[\s:：-]*([0-9]{1,3}(?:\.[0-9]+)?)\s*%/gi,
      /代码(?:审查|审阅|评审)(?:额度)?[\s:：-]*([0-9]{1,3}(?:\.[0-9]+)?)\s*%\s*(剩余|已使用|已用)/g,
      /代码(?:审查|审阅|评审)(?:额度)?[\s:：-]*(剩余|已使用|已用)[\s:：-]*([0-9]{1,3}(?:\.[0-9]+)?)\s*%/g,
    ];
    const amounts = new Set();
    for (const [index, pattern] of patterns.entries()) {
      for (const match of body.matchAll(pattern)) {
        const amount = Number(match[index % 2 === 0 ? 1 : 2]);
        const unit = match[index % 2 === 0 ? 2 : 1].toLowerCase();
        if (!Number.isFinite(amount) || amount < 0 || amount > 100) return null;
        amounts.add(['used', '已使用', '已用'].includes(unit) ? 100 - amount : amount);
      }
    }
    return amounts.size === 1 ? [...amounts][0] : null;
  };
  const get = async (path, token, selectAccount = true) => {
    // All paths are code constants below; credentials can only reach this exact origin.
    if (controller.signal.aborted || location.origin !== 'https://chatgpt.com') throw new Error('cancelled');
    const headers = { Accept: 'application/json' };
    if (token) {
      headers.Authorization = `Bearer ${token}`;
      if (selectAccount) headers['ChatGPT-Account-Id'] = config.expectedAccountId;
    }
    const requestController = new AbortController();
    const cancel = () => requestController.abort();
    controller.signal.addEventListener('abort', cancel, { once: true });
    let deadline;
    const timeout = new Promise((_, reject) => {
      deadline = setTimeout(() => {
        requestController.abort();
        reject(new Error('timeout'));
      }, token ? 7000 : 4000);
    });
    const request = async () => {
      const response = await fetch(`https://chatgpt.com${path}`, {
        method: 'GET', headers, credentials: 'same-origin', redirect: 'error',
        cache: 'no-store', signal: requestController.signal,
      });
      if (!response.ok) throw new Error('unavailable');
      const data = await response.json();
      if (!data || typeof data !== 'object' || Array.isArray(data)) throw new Error('invalid');
      // If an endpoint supplies identity, it must corroborate the account selected by the backend.
      const responseAccount = data.account_id ?? data.chatgpt_account_id;
      if (token && selectAccount && responseAccount != null && responseAccount !== config.expectedAccountId) throw new Error('identity');
      return data;
    };
    try {
      // Each endpoint settles independently so a hung history request preserves other data.
      return await Promise.race([request(), timeout]);
    } finally {
      clearTimeout(deadline);
      controller.signal.removeEventListener('abort', cancel);
    }
  };
  const run = async () => {
    const expected = typeof config.expectedAccount === 'string' ? config.expectedAccount.trim().toLowerCase() : '';
    if (!expected || typeof config.expectedAccountId !== 'string' || !config.expectedAccountId.trim()) {
      finish(empty('unavailable', '服务端尚未确认当前账号和工作区，暂不合并网页补充数据。'));
      return;
    }
    let session;
    try {
      session = await get('/api/auth/session');
    } catch {
      finish(empty('unavailable', '请在 Codex 用量网页完成登录，然后刷新使用统计。'));
      return;
    }
    const email = typeof session.user?.email === 'string' ? session.user.email.trim().toLowerCase() : '';
    if (!email || email !== expected) {
      finish(empty('unavailable', '网页登录账号与当前数据源不一致，请切换为相同账号后刷新。'));
      return;
    }
    const token = session.accessToken;
    if (typeof token !== 'string' || !token || token.length > 32768 || /\s/.test(token)) {
      finish(empty('unavailable', '网页登录已失效，请重新登录后刷新使用统计。'));
      return;
    }
    const end = new Date();
    const start = new Date(end);
    start.setUTCDate(start.getUTCDate() - 29);
    const range = `start_date=${start.toISOString().slice(0, 10)}&end_date=${end.toISOString().slice(0, 10)}&group_by=day`;
    const results = await Promise.allSettled([
      get('/backend-api/wham/usage', token),
      get(`/backend-api/wham/usage/daily-token-usage-breakdown?${range}`, token),
      get('/backend-api/wham/usage/credit-usage-events', token),
      // Check the webpage's default workspace separately before reading its visible widget.
      get('/backend-api/wham/usage', token, false),
    ]);
    if (results.some((result) => result.status === 'rejected' && result.reason?.message === 'identity')) {
      finish(empty('unavailable', '网页返回的工作区与当前数据源不一致，已停止合并网页补充数据。'));
      return;
    }
    const snapshot = empty('ready', null);
    snapshot.account = email;
    let sections = 0;
    const missing = [];
    const usage = results[0].status === 'fulfilled' ? results[0].value : null;
    const balance = number(usage?.credits?.balance);
    if (balance !== null && balance >= 0) snapshot.creditsRemaining = balance;
    const defaultUsage = results[3].status === 'fulfilled' ? results[3].value : null;
    if ((defaultUsage?.account_id ?? defaultUsage?.chatgpt_account_id) === config.expectedAccountId) {
      try { snapshot.codeReviewRemainingPercent = codeReview(); } catch { /* DOM not ready; preserve API data. */ }
    }
    if (snapshot.creditsRemaining !== null || snapshot.codeReviewRemainingPercent !== null) sections++;
    else missing.push('额度补充');

    const daily = results[1].status === 'fulfilled' ? results[1].value : null;
    if (Array.isArray(daily?.data) && daily.data.length <= 62) {
      const days = daily.data.map((row) => {
        const rowDate = date(row?.date);
        const values = row?.product_surface_usage_values;
        if (!rowDate || !values || typeof values !== 'object' || Array.isArray(values)) return null;
        const entries = Object.entries(values);
        if (entries.length > 64) return null;
        const amounts = entries.map(([service, amount]) => ({ service: name(service), amount: number(amount) }));
        if (amounts.some((entry) => entry.service === null || entry.amount === null)) return null;
        return { date: rowDate, amounts };
      });
      if (days.every((day) => day !== null)) {
        snapshot.usageBreakdown = days;
        // This endpoint can return credits despite "token" in its name. Never infer tokens.
        snapshot.usageUnit = name(daily.units);
        sections++;
      } else missing.push('每日用量');
    } else missing.push('每日用量');

    const credits = results[2].status === 'fulfilled' ? results[2].value : null;
    if (Array.isArray(credits?.data) && credits.data.length <= 1000) {
      const events = credits.data.map((row) => ({
        date: date(row?.date), service: name(row?.product_surface), credits: number(row?.credit_amount),
      }));
      if (events.every((event) => event.date !== null && event.service !== null && event.credits !== null)) {
        snapshot.creditEvents = events; // Negative amounts represent adjustments and must be preserved.
        sections++;
      } else missing.push('Credit 记录');
    } else missing.push('Credit 记录');

    if (!sections) {
      finish(empty('error', '网页补充接口不可用，请在用量网页检查登录状态和工作区权限。'));
      return;
    }
    if (missing.length) snapshot.message = `部分网页补充数据暂不可用：${missing.join('、')}。`;
    snapshot.updatedAt = new Date().toISOString();
    finish(snapshot);
  };
  const timeout = setTimeout(() => {
    finish(empty('unavailable', 'Codex 用量网页读取超时，请完成登录后刷新。'));
    controller.abort();
  }, 15000);
  run().catch(() => finish(empty('error', 'Codex 网页补充读取失败，请刷新使用统计。')))
    .finally(() => clearTimeout(timeout));
  return true;
})(/*__AGENTBAR_WEB_CONFIG_JSON__*/null);
