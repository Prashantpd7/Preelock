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
