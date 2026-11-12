import { afterEach, beforeEach, expect, it, vi } from 'vitest';
let session, local, rules, dynamicRules, message, storageListeners;
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
const storage = (getData, areaName = null) => ({
  get: async (keys) => {
    const data = getData();
    if (typeof keys === 'string') {
      return Object.hasOwn(data, keys) ? { [keys]: data[keys] } : {};
    }
    if (Array.isArray(keys)) {
      const out = {};
      for (const k of keys) {
        if (Object.hasOwn(data, k)) out[k] = data[k];
      }
      return out;
    }
    return { ...data };
  },
  set: async (p) => {
    const data = getData();
    const changes = {};
    for (const [k, v] of Object.entries(p)) {
      changes[k] = { oldValue: data[k], newValue: v };
      data[k] = v;
    }
    if (areaName === 'local') {
      for (const fn of storageListeners) fn(changes, 'local');
    }
  },
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
  dynamicRules = [];
  storageListeners = [];
  const event = () => ({ addListener: vi.fn() });
  vi.stubGlobal('chrome', {
    storage: {
      session: storage(() => session, 'session'),
      local: storage(() => local, 'local'),
      onChanged: {
        addListener: vi.fn((fn) => storageListeners.push(fn)),
      },
    },
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
      getDynamicRules: async () => dynamicRules,
      updateDynamicRules: async ({ removeRuleIds = [], addRules = [] }) => {
        dynamicRules = [
          ...dynamicRules.filter((r) => !removeRuleIds.includes(r.id)),
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
// M4 (v7): a failed sync no longer drops the rules while the stored
// policy's lease is still valid. During a bridge handover (GUI killed,
// background taking over) the bridge is briefly unreachable; dropping
// the rules then would open a blocking gap even though the policy the
// extension already holds is still live. The lease still bounds
// fail-open: once it expires, the rules are removed.
it('keeps rules while the stored policy lease is live on connection failure', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response(policy())),
  );
  await import('./worker.js');
  await send({ type: 'sync' });
  expect(rules.length).toBeGreaterThan(0);
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => {
      throw new Error('Failed to fetch');
    }),
  );
  await send({ type: 'sync' });
  expect(session.error).toContain('Open the desktop app');
  expect(rules.length).toBeGreaterThan(0);
  expect(session.applied_id).toBe('test-session');
});
it('releases rules on connection failure once the stored policy lease has expired', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response(policy())),
  );
  await import('./worker.js');
  await send({ type: 'sync' });
  expect(rules.length).toBeGreaterThan(0);
  session.policy.expires_at = Date.now() - 1;
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => {
      throw new Error('Failed to fetch');
    }),
  );
  await send({ type: 'sync' });
  expect(session.error).toContain('Open the desktop app');
  expect(rules).toEqual([]);
  expect(session.applied_id).toBe('');
});
it('recovers rules after a failed sync once the bridge answers again', async () => {
  let syncCalls = 0;
  let failSync = false;
  const request = vi.fn(async (_url, options) => {
    const body = JSON.parse(options.body);
    // The acknowledgement carries no `v`; only the sync request does.
    if (!('v' in body)) return response(policy());
    syncCalls += 1;
    if (failSync) throw new Error('Failed to fetch');
    return response({
      ...policy(),
      id: syncCalls === 1 ? 'test-session' : 'test-session-2',
    });
  });
  vi.stubGlobal('fetch', request);
  await import('./worker.js');
  await send({ type: 'sync' });
  expect(session.applied_id).toBe('test-session');
  expect(rules.length).toBeGreaterThan(0);
  failSync = true;
  await send({ type: 'sync' });
  expect(session.error).toContain('Open the desktop app');
  expect(rules.length).toBeGreaterThan(0);
  expect(session.applied_id).toBe('test-session');
  failSync = false;
  await send({ type: 'sync' });
  expect(session.applied_id).toBe('test-session-2');
  expect(rules.length).toBeGreaterThan(0);
  expect(session.error).toBe('');
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
  // The blocked page now carries the resolved cause as query params;
  // this test policy has no session_cause, so it falls back to unknown.
  expect(chrome.tabs.update).toHaveBeenCalledWith(10, {
    url: 'chrome-extension://test-extension/blocked.html?h=www.youtube.com&k=unknown',
  });
  chrome.tabs.update.mockClear();
  session.policy.expires_at = Date.now() - 1;
  onUpdated(11, { url: 'https://youtube.com/' }, { id: 11 });
  await send({ type: 'sync' });
  expect(chrome.tabs.update).not.toHaveBeenCalled();
});

it('reports channel_blocked for a DNR-blocked channel page tab', async () => {
  const channelPolicy = () => ({
    id: 'test-session',
    mode: 'blocklist',
    domains: [],
    youtube_blocked_channels: ['@blocked'],
    expires_at: Date.now() + 60000,
    observe_activity: true,
  });
  chrome.tabs.query = async () => [
    { id: 9, url: 'https://www.youtube.com/@blocked' },
  ];
  const request = vi.fn(async (_url, options) => {
    const body = JSON.parse(options.body);
    if (body.applied_id) expect(rules.length).toBeGreaterThan(0);
    return response(channelPolicy());
  });
  vi.stubGlobal('fetch', request);
  await import('./worker.js');
  await send({ type: 'sync' });
  const acknowledgement = JSON.parse(request.mock.calls.at(-1)[1].body);
  expect(acknowledgement.channel_blocked).toBe(true);
  expect(acknowledgement.active_domain).toBe('www.youtube.com');
  expect(JSON.stringify(request.mock.calls)).not.toContain('@blocked');
});
it('has no disconnect path: a disconnect message sends no goodbye', async () => {
  const request = vi.fn(async () => response(policy()));
  vi.stubGlobal('fetch', request);
  await import('./worker.js');
  // No handler answers a disconnect message, so the reply promise
  // never resolves: fire it without awaiting, flush async work, and
  // assert the bridge was never told this browser disconnected.
  void send({ type: 'disconnect' });
  await vi.advanceTimersByTimeAsync(100);
  const goodbyes = request.mock.calls.filter(
    ([, options]) => JSON.parse(options.body).disconnect === true,
  );
  expect(goodbyes).toHaveLength(0);
});

it('reports the extension version on every sync', async () => {
  const request = vi.fn(async () => response(policy()));
  vi.stubGlobal('fetch', request);
  await import('./worker.js');
  await send({ type: 'sync' });
  expect(request).toHaveBeenCalledTimes(2);
  for (const [, init] of request.mock.calls) {
    const body = JSON.parse(init.body);
    // Kept in sync with EXTENSION_VERSION in worker.js and browser.rs.
    expect(body.v).toBe(13);
  }
});

it('reports measured lecture playback from the focused window active tab', async () => {
  chrome.tabs.query = async () => [
    { id: 5, url: 'https://www.youtube.com/watch?v=lecture' },
  ];
  chrome.tabs.sendMessage = vi.fn((id, message, callback) => {
    if (message.type === 'preelock_media_state') {
      expect(id).toBe(5);
      callback({ playing: true });
    }
    return Promise.resolve();
  });
  const request = vi.fn(async () => response(policy()));
  vi.stubGlobal('fetch', request);
  await import('./worker.js');
  await send({ type: 'sync' });
  // The acknowledgement (last call) carries the activity report.
  const body = JSON.parse(request.mock.calls.at(-1)[1].body);
  expect(body.media_playing).toBe(true);
  // Only the measured boolean travels; never the page's URL.
  expect(JSON.stringify(request.mock.calls)).not.toContain('v=lecture');
});

it('reports no lecture playback when the video is paused or the tab cannot answer', async () => {
  chrome.tabs.query = async () => [
    { id: 6, url: 'https://www.youtube.com/watch?v=lecture' },
  ];
  chrome.tabs.sendMessage = vi.fn((id, message, callback) => {
    if (message.type === 'preelock_media_state') callback({ playing: false });
    return Promise.resolve();
  });
  const request = vi.fn(async () => response(policy()));
  vi.stubGlobal('fetch', request);
  await import('./worker.js');
  await send({ type: 'sync' });
  expect(JSON.parse(request.mock.calls.at(-1)[1].body).media_playing).toBe(
    false,
  );
  // A tab with no content script (a rejected message) is not playing.
  chrome.tabs.sendMessage = vi.fn((id, message, callback) => {
    if (message.type !== 'preelock_media_state') return Promise.resolve();
    chrome.runtime.lastError = { message: 'no receiver' };
    callback();
    return Promise.resolve();
  });
  await send({ type: 'sync' });
  const body = JSON.parse(request.mock.calls.at(-1)[1].body);
  expect(body.media_playing).toBe(false);
});

it('reports no lecture playback for a background browser window', async () => {
  chrome.windows.getLastFocused = async () => ({ id: 1, focused: false });
  chrome.tabs.query = async () => [
    { id: 7, url: 'https://www.youtube.com/watch?v=lecture' },
  ];
  chrome.tabs.sendMessage = vi.fn(() => Promise.resolve());
  const request = vi.fn(async () => response(policy()));
  vi.stubGlobal('fetch', request);
  await import('./worker.js');
  await send({ type: 'sync' });
  const body = JSON.parse(request.mock.calls.at(-1)[1].body);
  expect(body.browser_focused).toBe(false);
  expect(body.media_playing).toBeUndefined();
  // Playback is never even queried for an unfocused window.
  expect(
    chrome.tabs.sendMessage.mock.calls.filter(
      ([, message]) => message.type === 'preelock_media_state',
    ),
  ).toHaveLength(0);
});

it('uses the chosen Comet identity when Chrome is installed too', async () => {
  local.browser_choice = 'ai.perplexity.comet';
  const request = vi.fn(async () => response(null));
  vi.stubGlobal('fetch', request);
  await import('./worker.js');
  await send({ type: 'sync' });
  expect(JSON.parse(request.mock.calls[0][1].body).browser).toBe(
    'ai.perplexity.comet',
  );
});

it('passes the resolved block cause to the blocked page', async () => {
  const endsAt = Date.now() + 2 * 3600 * 1000;
  const causalPolicy = () => ({
    id: 'test-session',
    mode: 'blocklist',
    domains: ['example.com'],
    extra_blocked: ['youtube.com'],
    rule_causes: [
      {
        rule: 'youtube.com',
        cause: { kind: 'timed_lock', ends_at_ms: endsAt },
      },
    ],
    session_cause: { kind: 'session', title: 'Deep work' },
    expires_at: Date.now() + 60000,
    observe_activity: true,
  });
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response(causalPolicy())),
  );
  await import('./worker.js');
  await send({ type: 'sync' });
  const onUpdated = chrome.tabs.onUpdated.addListener.mock.calls[0][0];
  // Extra-rule hit: the per-rule cause (timed lock) wins over the session cause.
  onUpdated(20, { url: 'https://www.youtube.com/watch?v=x' }, { id: 20 });
  await send({ type: 'sync' }); // drains the worker queue
  expect(chrome.tabs.update).toHaveBeenCalledWith(20, {
    url:
      'chrome-extension://test-extension/blocked.html' +
      `?h=www.youtube.com&k=timed_lock&e=${endsAt}`,
  });
  chrome.tabs.update.mockClear();
  // Session domain rule: the session cause applies.
  onUpdated(21, { url: 'https://example.com/' }, { id: 21 });
  await send({ type: 'sync' });
  expect(chrome.tabs.update).toHaveBeenCalledWith(21, {
    url:
      'chrome-extension://test-extension/blocked.html' +
      '?h=example.com&k=session&t=Deep+work',
  });
});

// The Watch Later exception must survive the navigation listener too:
// DNR cannot stop a service-worker-served page, so the tab replacement
// is what actually blocks YouTube. A direct URL and a link click both
// arrive as an onUpdated navigation to the same route.
const watchLaterPolicy = () => ({
  id: 'test-session',
  mode: 'allowlist',
  domains: ['github.com'],
  youtube_allow_watch_later: true,
  expires_at: Date.now() + 60000,
  observe_activity: true,
});
it('opens the Watch Later playlist but replaces every other YouTube route', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response(watchLaterPolicy())),
  );
  await import('./worker.js');
  await send({ type: 'sync' });
  chrome.tabs.update.mockClear();
  const onUpdated = chrome.tabs.onUpdated.addListener.mock.calls[0][0];
  // Direct URL (typed or pasted) and a clicked link: both land here.
  onUpdated(30, { url: 'https://www.youtube.com/playlist?list=WL' });
  onUpdated(
    31,
    { status: 'loading' },
    {
      id: 31,
      pendingUrl: 'https://m.youtube.com/playlist?list=WL',
    },
  );
  await send({ type: 'sync' });
  expect(chrome.tabs.update).not.toHaveBeenCalled();
  // No redirect loop: the block page itself is never re-blocked.
  onUpdated(30, {
    url: `chrome-extension://test-extension/blocked.html?h=www.youtube.com`,
  });
  await send({ type: 'sync' });
  expect(chrome.tabs.update).not.toHaveBeenCalled();
  // Homepage and general feeds redirect to Watch Later so navigation never
  // dead-ends on blocked.html, while every other disallowed route is blocked.
  onUpdated(40, { url: 'https://www.youtube.com/' });
  onUpdated(39, { url: 'https://www.youtube.com/feed/subscriptions' });
  const blocked = [
    'https://www.youtube.com/shorts/abc',
    'https://www.youtube.com/results?search_query=cats',
    'https://www.youtube.com/watch?v=x',
    'https://www.youtube.com/playlist?list=PL123',
    'https://www.youtube.com/playlist?list=WLxyz',
    'https://www.youtube.com/@channel',
  ];
  blocked.forEach((url, i) => onUpdated(41 + i, { url }));
  await send({ type: 'sync' });
  expect(chrome.tabs.update).toHaveBeenCalledWith(40, {
    url: 'https://www.youtube.com/playlist?list=WL',
  });
  expect(chrome.tabs.update).toHaveBeenCalledWith(39, {
    url: 'https://www.youtube.com/playlist?list=WL',
  });
  for (let i = 0; i < blocked.length; i++) {
    expect(chrome.tabs.update).toHaveBeenCalledWith(41 + i, {
      url: expect.stringContaining(
        'chrome-extension://test-extension/blocked.html',
      ),
    });
  }
  // A Watch Later *video* route now opens so the content script can
  // verify that the video is really in the playlist; the URL alone is
  // not a permission.
  onUpdated(48, { url: 'https://www.youtube.com/watch?v=x&list=WL' });
  await send({ type: 'sync' });
  expect(chrome.tabs.update.mock.calls.map(([id]) => id)).not.toContain(48);
});
it('leaves an already-open Watch Later tab alone and redirects open YouTube Home when a session starts', async () => {
  chrome.tabs.query = async () => [
    { id: 50, url: 'https://www.youtube.com/playlist?list=WL' },
    { id: 51, url: 'https://www.youtube.com/' },
  ];
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response(watchLaterPolicy())),
  );
  await import('./worker.js');
  await send({ type: 'sync' });
  expect(chrome.tabs.update).toHaveBeenCalledTimes(1);
  expect(chrome.tabs.update).toHaveBeenCalledWith(51, {
    url: 'https://www.youtube.com/playlist?list=WL',
  });
});
it('drops the Watch Later exception when the session policy ends', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response(watchLaterPolicy())),
  );
  await import('./worker.js');
  await send({ type: 'sync' });
  expect(rules.some((r) => /list=WL/.test(r.condition.regexFilter || ''))).toBe(
    true,
  );
  // The engine publishes no policy once the session completes.
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response(null)),
  );
  await send({ type: 'sync' });
  expect(rules).toEqual([]);
  expect(session.policy).toBeNull();
  // With no live policy nothing is replaced, so YouTube blocking is back
  // to its out-of-session state.
  chrome.tabs.update.mockClear();
  const onUpdated = chrome.tabs.onUpdated.addListener.mock.calls[0][0];
  onUpdated(60, { url: 'https://www.youtube.com/playlist?list=WL' });
  onUpdated(61, { url: 'https://www.youtube.com/' });
  await send({ type: 'sync' });
  expect(chrome.tabs.update).not.toHaveBeenCalled();
});

// The content script cannot import rules.js, so every decision it needs
// is answered by the worker: the policy, each SPA route, each video
// (channel + verified Watch Later membership), and the blocked-screen
// wording. These tests exercise those handlers directly.
async function sendFromTab(value, tab) {
  return new Promise((resolve) =>
    message(value, { id: 'test-extension', tab }, resolve),
  );
}
it('answers the content script with routes and verified membership', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response(watchLaterPolicy())),
  );
  await import('./worker.js');
  await send({ type: 'sync' });
  const policyResponse = await send({ type: 'youtube_policy' });
  expect(policyResponse.policy.youtube_allow_watch_later).toBe(true);
  // Ordinary YouTube browsing is closed, so the script may redirect the
  // homepage and hide the feeds.
  expect(policyResponse.browsingClosed).toBe(true);
  // Routes: the playlist opens, a channel page does not.
  expect(
    await send({
      type: 'youtube_page_check',
      url: 'https://www.youtube.com/playlist?list=WL',
    }),
  ).toEqual({ blocked: false });
  expect(
    await send({
      type: 'youtube_page_check',
      url: 'https://www.youtube.com/@channel',
    }),
  ).toEqual({ blocked: true });
  const wlVideo = 'https://www.youtube.com/watch?v=abc123&list=WL';
  // A video is decided by membership, never by the list=WL parameter.
  expect(
    await send({
      type: 'youtube_check',
      url: wlVideo,
      channel: '@other',
      watchLaterMember: true,
    }),
  ).toEqual({ blocked: false, reason: 'watch_later' });
  expect(
    await send({
      type: 'youtube_check',
      url: wlVideo,
      channel: '@other',
      watchLaterMember: null,
    }),
  ).toEqual({ blocked: false, reason: null, pending: true });
  expect(
    await send({
      type: 'youtube_check',
      url: wlVideo,
      channel: '@other',
      watchLaterMember: false,
    }),
  ).toEqual({ blocked: true, reason: 'watch_later_unverified' });
});
it('replaces blocked YouTube content with the matching blocked screen', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () =>
      response({
        ...watchLaterPolicy(),
        youtube_blocked_channels: ['@blocked'],
      }),
    ),
  );
  await import('./worker.js');
  await send({ type: 'sync' });
  chrome.tabs.update.mockClear();
  const videoTab = {
    id: 70,
    url: 'https://www.youtube.com/watch?v=abc123&list=WL',
  };
  await sendFromTab(
    {
      type: 'youtube_block',
      url: videoTab.url,
      channel: '@other',
      watchLaterMember: false,
    },
    videoTab,
  );
  expect(chrome.tabs.update).toHaveBeenCalledWith(70, {
    url: 'chrome-extension://test-extension/blocked.html?k=youtube&m=content',
  });
  // A channel block gets the channel wording.
  const channelTab = { id: 71, url: 'https://www.youtube.com/@blocked' };
  await sendFromTab(
    { type: 'youtube_block', url: channelTab.url, channel: null },
    channelTab,
  );
  expect(chrome.tabs.update).toHaveBeenCalledWith(71, {
    url: 'chrome-extension://test-extension/blocked.html?k=youtube&m=channel',
  });
});
it('leaves blocked content alone once the session has ended', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response(null)),
  );
  await import('./worker.js');
  await send({ type: 'sync' });
  chrome.tabs.update.mockClear();
  const tab = { id: 72, url: 'https://www.youtube.com/watch?v=abc123' };
  await sendFromTab(
    {
      type: 'youtube_block',
      url: tab.url,
      channel: null,
      watchLaterMember: false,
    },
    tab,
  );
  expect(chrome.tabs.update).not.toHaveBeenCalled();
});
it('echos videoId, learns UC-id aliases across checks, and fails closed when channel rules require identity', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () =>
      response({
        ...watchLaterPolicy(),
        youtube_blocked_channels: ['@blocked'],
      }),
    ),
  );
  await import('./worker.js');
  await send({ type: 'sync' });
  const ucId = 'UCBlockedChannelId123456';
  // First observation reports both the UC channel id and the @handle:
  // blocked immediately, and aliases are learned for subsequent lookups.
  expect(
    await send({
      type: 'youtube_check',
      url: 'https://www.youtube.com/watch?v=vid1',
      videoId: 'vid1',
      channels: [ucId, '@blocked'],
    }),
  ).toEqual({
    videoId: 'vid1',
    blocked: true,
    reason: 'blocked_channel',
  });
  // Later observation reports only the UC id (e.g., an embed or minimal
  // player response without an owner handle link): alias expansion still blocks.
  expect(
    await send({
      type: 'youtube_check',
      url: 'https://www.youtube.com/watch?v=vid2&list=WL',
      videoId: 'vid2',
      channels: [ucId],
      watchLaterMember: true,
    }),
  ).toEqual({
    videoId: 'vid2',
    blocked: true,
    reason: 'blocked_channel',
  });
  // When no channel identity arrives on a Watch Later video while blocked
  // channels exist, the check stays pending first and then fails closed.
  expect(
    await send({
      type: 'youtube_check',
      url: 'https://www.youtube.com/watch?v=vid3&list=WL',
      videoId: 'vid3',
      channels: [],
      watchLaterMember: true,
      unidentified: false,
    }),
  ).toEqual({
    videoId: 'vid3',
    blocked: false,
    reason: null,
    pending: true,
  });
  expect(
    await send({
      type: 'youtube_check',
      url: 'https://www.youtube.com/watch?v=vid3&list=WL',
      videoId: 'vid3',
      channels: [],
      watchLaterMember: true,
      unidentified: true,
    }),
  ).toEqual({
    videoId: 'vid3',
    blocked: true,
    reason: 'channel_unidentified',
  });
  chrome.tabs.update.mockClear();
  const tab = {
    id: 73,
    url: 'https://www.youtube.com/watch?v=vid3&list=WL',
  };
  await sendFromTab(
    {
      type: 'youtube_block',
      url: tab.url,
      channels: [],
      watchLaterMember: true,
      unidentified: true,
    },
    tab,
  );
  expect(chrome.tabs.update).toHaveBeenCalledWith(73, {
    url: 'chrome-extension://test-extension/blocked.html?k=youtube&m=content',
  });
});

it('blocks aliased /@handle and /@handle/live pages in youtube_page_check and youtube_block without learning ambiguous multi-channel tuples', async () => {
  const ucBlocked = 'UCHnyfMqiRRG1u-2MsSQLbXA';
  const ucOther = 'UCYo1b5k3x9n7pQ2rStUvWx1';
  vi.stubGlobal(
    'fetch',
    vi.fn(async () =>
      response({
        ...policy(),
        mode: 'blocklist',
        domains: [],
        youtube_blocked_channels: [ucBlocked],
      }),
    ),
  );
  await import('./worker.js');
  await send({ type: 'sync' });

  // Ambiguous array with two UC IDs must NOT poison alias map.
  await send({
    type: 'youtube_check',
    url: 'https://www.youtube.com/watch?v=ambig01',
    videoId: 'ambig01',
    channels: [ucBlocked, ucOther, '@innocent'],
  });
  expect(
    await send({
      type: 'youtube_page_check',
      url: 'https://www.youtube.com/@innocent/live',
    }),
  ).toEqual({ blocked: false });

  // Clean 1 UC ID + 1 @handle observation learns bidirectional alias.
  expect(
    await send({
      type: 'youtube_check',
      url: 'https://www.youtube.com/watch?v=ver01',
      videoId: 'ver01',
      channels: [ucBlocked, '@veritasium'],
    }),
  ).toEqual({
    videoId: 'ver01',
    blocked: true,
    reason: 'blocked_channel',
  });

  expect(
    await send({
      type: 'youtube_page_check',
      url: 'https://www.youtube.com/@veritasium',
    }),
  ).toEqual({ blocked: true });
  expect(
    await send({
      type: 'youtube_page_check',
      url: 'https://www.youtube.com/@veritasium/live',
    }),
  ).toEqual({ blocked: true });

  chrome.tabs.update.mockClear();
  const liveTab = {
    id: 88,
    url: 'https://www.youtube.com/@veritasium/live',
  };
  await sendFromTab(
    {
      type: 'youtube_block',
      url: liveTab.url,
      channels: [],
    },
    liveTab,
  );
  expect(chrome.tabs.update).toHaveBeenCalledWith(88, {
    url: 'chrome-extension://test-extension/blocked.html?k=youtube&m=channel',
  });
});

it('enforces local Website Controls independently of desktop sessions and combines additively with active policies', async () => {
  // Disconnected state: no token, no session policy.
  local = {
    websiteControls: {
      blockAdultWebsites: true,
      blockDistractingWebsites: true,
    },
  };
  chrome.tabs.query = async () => [
    { id: 90, url: 'https://www.instagram.com/reels/' },
    { id: 91, url: 'https://www.youtube.com/' },
    { id: 92, url: 'https://web.whatsapp.com/' },
    { id: 93, url: 'https://www.pornhub.com/' },
  ];
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response(null)),
  );
  await import('./worker.js');
  await send({ type: 'reconcile_website_controls' });

  // Dynamic rules installed for both local categories; session rules empty.
  expect(rules).toEqual([]);
  expect(dynamicRules.some((r) => r.id === 100001 && r.priority === 6)).toBe(
    true,
  );
  expect(dynamicRules.some((r) => r.id >= 101000 && r.priority === 6)).toBe(
    true,
  );

  // Open distracting & adult tabs immediately replaced; YouTube & WhatsApp untouched.
  expect(chrome.tabs.update).toHaveBeenCalledWith(90, {
    url: 'chrome-extension://test-extension/blocked.html?h=www.instagram.com&k=website_distracting',
  });
  expect(chrome.tabs.update).toHaveBeenCalledWith(93, {
    url: 'chrome-extension://test-extension/blocked.html?h=www.pornhub.com&k=website_adult',
  });
  expect(chrome.tabs.update.mock.calls.map(([id]) => id)).not.toContain(91);
  expect(chrome.tabs.update.mock.calls.map(([id]) => id)).not.toContain(92);

  // Idempotent sync must NOT churn or duplicate dynamic rules.
  const updateDynamicSpy = vi.spyOn(
    chrome.declarativeNetRequest,
    'updateDynamicRules',
  );
  await send({ type: 'reconcile_website_controls' });
  expect(updateDynamicSpy).not.toHaveBeenCalled();

  // Disconnected tabs.onUpdated still enforces local categories.
  chrome.tabs.update.mockClear();
  const onUpdated = chrome.tabs.onUpdated.addListener.mock.calls[0][0];
  onUpdated(94, { url: 'https://m.facebook.com/' });
  onUpdated(95, { url: 'https://www.netflix.com/browse' });
  onUpdated(96, { url: 'https://www.youtube.com/watch?v=x' });
  await send({ type: 'sync' });
  expect(chrome.tabs.update).toHaveBeenCalledWith(94, {
    url: 'chrome-extension://test-extension/blocked.html?h=m.facebook.com&k=website_distracting',
  });
  expect(chrome.tabs.update).toHaveBeenCalledWith(95, {
    url: 'chrome-extension://test-extension/blocked.html?h=www.netflix.com&k=website_distracting',
  });
  expect(chrome.tabs.update.mock.calls.map(([id]) => id)).not.toContain(96);

  // Now connect desktop session with an allowlist that includes instagram.com:
  // additive combined precedence means local Distracting toggle (priority 6) still blocks Instagram!
  chrome.tabs.update.mockClear();
  local.token = 'a'.repeat(64);
  local.client = 'test-client';
  vi.stubGlobal(
    'fetch',
    vi.fn(async () =>
      response({
        id: 'allow-ig-session',
        mode: 'allowlist',
        domains: ['instagram.com', 'github.com'],
        expires_at: Date.now() + 60000,
      }),
    ),
  );
  await send({ type: 'sync' });
  expect(chrome.tabs.update).toHaveBeenCalledWith(90, {
    url: 'chrome-extension://test-extension/blocked.html?h=www.instagram.com&k=website_distracting',
  });
  expect(rules.length).toBeGreaterThan(0);
  expect(dynamicRules.some((r) => r.id === 100001)).toBe(true);

  // Ending desktop session removes ONLY session rules; local Website Controls persist.
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response(null)),
  );
  await send({ type: 'sync' });
  expect(rules).toEqual([]);
  expect(dynamicRules.some((r) => r.id === 100001)).toBe(true);
  expect(dynamicRules.some((r) => r.id >= 101000)).toBe(true);
  expect(local.websiteControls).toEqual({
    blockAdultWebsites: true,
    blockDistractingWebsites: true,
  });

  // Turning off only Distracting removes 100001 while leaving Adult rules intact.
  await chrome.storage.local.set({
    websiteControls: {
      blockAdultWebsites: true,
      blockDistractingWebsites: false,
    },
  });
  await send({ type: 'reconcile_website_controls' });
  expect(dynamicRules.some((r) => r.id === 100001)).toBe(false);
  expect(dynamicRules.some((r) => r.id >= 101000)).toBe(true);
});

it('verifies DNR rule installation in packed CWS builds without testMatchOutcome', async () => {
  delete chrome.declarativeNetRequest.testMatchOutcome;
  let first = true;
  const request = vi.fn(async () => {
    const test_id = first ? 'cws-test' : '';
    first = false;
    return { ok: true, json: async () => ({ policy: null, test_id }) };
  });
  vi.stubGlobal('fetch', request);
  await import('./worker.js');
  await send({ type: 'sync' });
  expect(
    request.mock.calls.some(
      ([, opts]) =>
        JSON.parse(opts.body).test_result ===
        'Passed · Rule engine verified (live matcher checks unpacked only)',
    ),
  ).toBe(true);
  expect(rules).toEqual([]);
});

it('auto-generates missing client UUID and queues explicit sync after an in-flight poll', async () => {
  local.token = 'c'.repeat(64);
  delete local.client;
  let releaseFirst;
  let calls = 0;
  const seenTokens = [];
  const seenClients = [];
  const fetchMock = vi.fn(async (_url, opts) => {
    calls += 1;
    const auth = opts.headers.Authorization;
    seenTokens.push(auth);
    const parsed = JSON.parse(opts.body);
    seenClients.push(parsed.client);
    if (calls === 1) {
      await new Promise((r) => {
        releaseFirst = r;
      });
      return response(null, 403);
    }
    return response(null, 200);
  });
  vi.stubGlobal('fetch', fetchMock);
  await import('./worker.js');
  await vi.waitFor(() => expect(calls).toBe(1));

  // User saves a fresh valid token while first poll with old token is still pending.
  local.token = 'd'.repeat(64);
  const explicitSync = send({ type: 'sync', force: true });
  releaseFirst();
  await explicitSync;

  expect(seenClients[0]).toBeTruthy();
  expect(local.client).toBe(seenClients[0]);
  expect(seenTokens).toContain(`Bearer ${'d'.repeat(64)}`);
  expect(session.error).toBe('');
});

it('evaluates videoMeta for youtube_block_entertainment and opens the entertainment block screen', async () => {
  local.token = 'a'.repeat(64);
  local.client = 'worker-client-ent';
  session.policy = {
    mode: 'blocklist',
    domains: [],
    youtube_block_entertainment: true,
    expires_at: Date.now() + 30_000,
  };
  await import('./worker.js');

  expect(
    await send({
      type: 'youtube_check',
      url: 'https://www.youtube.com/watch?v=edu123',
      videoId: 'edu123',
      channels: [],
      videoMeta: { title: 'Operating Systems Lecture', category: 'Education' },
    }),
  ).toEqual({ videoId: 'edu123', blocked: false, reason: null });

  expect(
    await send({
      type: 'youtube_check',
      url: 'https://www.youtube.com/watch?v=fun123',
      videoId: 'fun123',
      channels: [],
      videoMeta: { title: 'Standup Comedy & Prank Vlog', category: 'Comedy' },
    }),
  ).toEqual({ videoId: 'fun123', blocked: true, reason: 'entertainment' });

  chrome.tabs.update.mockClear();
  await sendFromTab(
    {
      type: 'youtube_block',
      url: 'https://www.youtube.com/watch?v=fun123',
      videoId: 'fun123',
      channels: [],
      videoMeta: { title: 'Standup Comedy & Prank Vlog', category: 'Comedy' },
    },
    { id: 99, url: 'https://www.youtube.com/watch?v=fun123' },
  );
  expect(chrome.tabs.update).toHaveBeenCalledWith(99, {
    url: 'chrome-extension://test-extension/blocked.html?k=youtube&m=entertainment',
  });
});
