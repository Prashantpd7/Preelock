import { afterEach, beforeEach, expect, it, vi } from 'vitest';
let session, local, rules, message;
const policy = () => ({
  id: 'test-session',
  mode: 'allowlist',
  domains: ['example.com'],
  expires_at: Date.now() + 60000,
  observe_activity: true,
});
const response = (p, status = 200) => ({
  ok: status === 200,
  status,
  json: async () => ({ policy: p }),
});
const storage = (data) => ({
  get: async () => ({ ...data }),
  set: async (p) => Object.assign(data, p),
});
async function send(value) {
  return new Promise((resolve) =>
    message(value, { id: 'test-extension' }, resolve),
  );
}
beforeEach(() => {
  vi.resetModules();
  vi.useFakeTimers();
  session = {};
  local = { token: 'a'.repeat(64), client: 'test-client' };
  rules = [];
  const event = () => ({ addListener: vi.fn() });
  vi.stubGlobal('chrome', {
    storage: { session: storage(session), local: storage(local) },
    runtime: {
      id: 'test-extension',
      getURL: (path) => `chrome-extension://test-extension/${path}`,
      onInstalled: event(),
      onStartup: event(),
      onMessage: {
        addListener: (fn) => {
          message = fn;
        },
      },
    },
    action: { setBadgeText: vi.fn(), setBadgeBackgroundColor: vi.fn() },
    alarms: { create: vi.fn(), onAlarm: event() },
    windows: { getLastFocused: async () => ({ id: 1, focused: true }) },
    tabs: {
      onUpdated: event(),
      update: vi.fn(async () => {}),
      query: async () => [
        { url: 'https://example.com/private/path?secret=never-send' },
      ],
    },
    declarativeNetRequest: {
      getSessionRules: async () => rules,
      updateSessionRules: async ({ removeRuleIds = [], addRules = [] }) => {
        rules = [
          ...rules.filter((r) => !removeRuleIds.includes(r.id)),
          ...addRules,
        ];
      },
    },
  });
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
it('installs before acknowledgement and sends only hostname metadata', async () => {
  const request = vi.fn(async (_url, options) => {
    const body = JSON.parse(options.body);
    if (body.applied_id) expect(rules.length).toBeGreaterThan(0);
    return response(policy());
  });
  vi.stubGlobal('fetch', request);
  await import('./worker.js');
  await send({ type: 'sync' });
  expect(session.applied_id).toBe('test-session');
  expect(rules.length).toBeGreaterThan(0);
  const acknowledgement = JSON.parse(request.mock.calls.at(-1)[1].body);
  expect(acknowledgement.active_domain).toBe('example.com');
  expect(JSON.stringify(request.mock.calls)).not.toContain('private/path');
});
it('clears installed rules when acknowledgement is rejected', async () => {
  let n = 0;
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response(policy(), ++n === 1 ? 200 : 403)),
  );
  await import('./worker.js');
  await send({ type: 'sync' });
  expect(rules).toEqual([]);
  expect(session.applied_id).toBe('');
  expect(session.error).toContain('Connection code rejected');
});
it('releases rules on connection failure and supports deliberate emergency release', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response(policy())),
  );
  await import('./worker.js');
  await send({ type: 'sync' });
  await send({ type: 'release', confirmation: 'END FOCUS' });
  expect(local.disabled).toBe(true);
  expect(rules).toEqual([]);
  local.disabled = false;
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => {
      throw new Error('Failed to fetch');
    }),
  );
  await send({ type: 'sync' });
  expect(session.error).toContain('Open the desktop app');
  expect(rules).toEqual([]);
});
it('runs requested browser checks and acknowledges the result without leaving rules', async () => {
  const matches = [1, null, 4, 5, 2, 4, 6, 2];
  chrome.declarativeNetRequest.testMatchOutcome = vi.fn(async () => ({
    matchedRules: [matches.shift()]
      .filter(Boolean)
      .map((ruleId) => ({ ruleId })),
  }));
  let first = true;
  const request = vi.fn(async () => {
    const test_id = first ? 'desktop-test' : '';
    first = false;
    return { ok: true, json: async () => ({ policy: null, test_id }) };
  });
  vi.stubGlobal('fetch', request);
  await import('./worker.js');
  await send({ type: 'sync' });
  expect(
    request.mock.calls.some(
      ([, opts]) =>
        JSON.parse(opts.body).test_result === 'Passed · 8 browser checks',
    ),
  ).toBe(true);
  expect(rules).toEqual([]);
});

it('replaces cached blocked normal and incognito tabs and repairs missing rules', async () => {
  chrome.tabs.query = async () => [
    { id: 1, url: 'https://www.youtube.com/watch?v=test' },
    { id: 2, url: 'https://youtube.com/', incognito: true },
    { id: 3, url: 'https://example.com/' },
    { id: 4, url: 'http://127.0.0.1:47832/sync' },
  ];

  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response(policy())),
  );
  await import('./worker.js');
  await send({ type: 'sync' });
  expect(chrome.tabs.update.mock.calls.map(([id]) => id)).toEqual([1, 2]);
  rules = [];
  await send({ type: 'sync' });
  expect(rules.length).toBeGreaterThan(0);
  expect(chrome.tabs.update).toHaveBeenCalledTimes(4);
});

it('blocks cached navigations after activation without waiting for network requests', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response(policy())),
  );
  await import('./worker.js');
  await send({ type: 'sync' });
  const onUpdated = chrome.tabs.onUpdated.addListener.mock.calls[0][0];
  onUpdated(10, { url: 'https://www.youtube.com/watch?v=cached' }, { id: 10 });
  await send({ type: 'sync' }); // drains the worker queue
  expect(chrome.tabs.update).toHaveBeenCalledWith(10, {
    url: 'chrome-extension://test-extension/blocked.html',
  });
  chrome.tabs.update.mockClear();
  session.policy.expires_at = Date.now() - 1;
  onUpdated(11, { url: 'https://youtube.com/' }, { id: 11 });
  await send({ type: 'sync' });
  expect(chrome.tabs.update).not.toHaveBeenCalled();
});

it('sends an authenticated goodbye when disconnecting', async () => {
  const request = vi.fn(async () => response(policy()));
  vi.stubGlobal('fetch', request);
  await import('./worker.js');
  const result = await send({ type: 'disconnect' });
  expect(result).toEqual({ ok: true });
  const goodbyes = request.mock.calls.filter(
    ([, options]) => JSON.parse(options.body).disconnect === true,
  );
  expect(goodbyes).toHaveLength(1);
  const [url, options] = goodbyes[0];
  expect(url).toBe('http://127.0.0.1:47832/sync');
  expect(options.headers.Authorization).toBe(`Bearer ${'a'.repeat(64)}`);
  const body = JSON.parse(options.body);
  expect(body.disconnect).toBe(true);
  expect(body.client).toBe('test-client');
  expect(body.applied_id).toBe('');
  expect(body.v).toBe(3);
});

it('reports the extension version on every sync', async () => {
  const request = vi.fn(async () => response(policy()));
  vi.stubGlobal('fetch', request);
  await import('./worker.js');
  await send({ type: 'sync' });
  const body = JSON.parse(request.mock.calls[0][1].body);
  expect(body.v).toBe(3);
});

it('uses the chosen Comet identity when Chrome is installed too', async () => {
  local.browser_choice = 'ai.perplexity.comet';
  const request = vi.fn(async () => response(null));
  vi.stubGlobal('fetch', request);
  await import('./worker.js');
  await send({ type: 'sync' });
  expect(JSON.parse(request.mock.calls[0][1].body).browser).toBe('ai.perplexity.comet');
});
