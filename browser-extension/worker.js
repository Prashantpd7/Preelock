import {
  blockDetail,
  channelBlockReason,
  compileRules,
  leaseValid,
  ruleMatches,
  searchPageQuery,
  searchQueryBlocked,
  urlBlockedByPolicy,
  youtubeWatchDecision,
} from './rules.js';
// The blocked page explains WHY: the worker already matched the
// rule, so it passes the resolved cause as query params (?k=kind,
// ?t=title, ?e=ends_at_ms). blocked.js renders them in plain words;
// an unknown or missing cause falls back to a generic line.
function causeForDetail(detail, policy) {
  if (detail.mechanism === 'extra' && detail.rule) {
    const hit = (policy.rule_causes || []).find(
      (rc) => rc && rc.rule === detail.rule,
    );
    if (hit && hit.cause) return hit.cause;
  }
  if (policy.session_cause) return policy.session_cause;
  return { kind: 'unknown' };
}
function blockedPageUrl(url, policy) {
  const detail = policy ? blockDetail(url, policy) : { blocked: false };
  const cause = causeForDetail(detail, policy || {});
  const params = new URLSearchParams();
  try {
    params.set('h', new URL(url).hostname);
  } catch {
    // Non-URL input: no host param.
  }
  params.set('k', cause.kind || 'unknown');
  if (cause.title) params.set('t', cause.title);
  if (cause.ends_at_ms) params.set('e', String(cause.ends_at_ms));
  return chrome.runtime.getURL('blocked.html') + '?' + params.toString();
}
import { browserIdentity } from './identity.js';
import { selfTest } from './self-test.js';
// Bump this every time extension behavior changes. The desktop app
// compares it against its own copy to spot extension copies that were
// never reloaded after an app update.
const EXTENSION_VERSION = 6;
let serial = Promise.resolve();
const enqueue = (work) => {
  serial = serial.then(work, work);
  return serial;
};
// Coalesce timer/alarm triggers while a bridge request is pending.
let pendingSync;
function requestSync() {
  if (!pendingSync)
    pendingSync = enqueue(sync).finally(() => {
      pendingSync = null;
    });
  return pendingSync;
}
async function removeRules() {
  const rules = await chrome.declarativeNetRequest.getSessionRules();
  await chrome.declarativeNetRequest.updateSessionRules({
    removeRuleIds: rules.map((r) => r.id),
  });
  await chrome.storage.session.set({
    policy: null,
    applied_id: '',
    channel_blocked_tabs: {},
  });
}
// Tabs the worker sent to the block page for a channel rule (a
// content-script block, or a DNR-blocked channel/surface page the
// worker replaced). The block page URL itself carries no channel, so
// activity() needs this map to report channel_blocked for it. Keys
// are tab ids; entries are cleared when the tab navigates elsewhere,
// closes, or the policy goes away (see removeRules).
async function markChannelBlocked(tabId) {
  if (tabId == null) return;
  const { channel_blocked_tabs = {} } = await chrome.storage.session.get(
    'channel_blocked_tabs',
  );
  await chrome.storage.session.set({
    channel_blocked_tabs: { ...channel_blocked_tabs, [tabId]: true },
  });
}
async function clearChannelBlocked(tabId) {
  if (tabId == null) return;
  const { channel_blocked_tabs = {} } = await chrome.storage.session.get(
    'channel_blocked_tabs',
  );
  if (!(tabId in channel_blocked_tabs)) return;
  const next = { ...channel_blocked_tabs };
  delete next[tabId];
  await chrome.storage.session.set({ channel_blocked_tabs: next });
}
// True if the new policy would block this URL. Used to reload already-open
// tabs when a session starts, so a page that was open beforehand (e.g.
// YouTube) is stopped instead of silently continuing. Full-URL decision
// (host, wildcard, path, keyword search pages, adult overlay) lives in
// rules.js so it cannot drift from the DNR rules.
function tabBlockedBy(url, policy) {
  return urlBlockedByPolicy(url, policy);
}
// DNR cannot block a page served entirely from a website service worker.
// Replace blocked top-level tabs too, without clearing browsing data or
// storing their URLs. This also stops already-playing cached video.
async function blockTab(tab, policy) {
  const url = tab.pendingUrl || tab.url;
  if (tab?.id != null && tabBlockedBy(url, policy)) {
    // Remember channel-rule blocks for the heartbeat: the block page
    // URL carries no channel, and a DNR error page keeps the blocked
    // URL (activity() recomputes that case from the URL instead).
    if (channelBlockReason(url, policy)) await markChannelBlocked(tab.id);
    await chrome.tabs
      .update(tab.id, { url: blockedPageUrl(url, policy) })
      .catch(() => {}); // A tab may close between query and update.
  }
}
async function blockOpenTabs(policy) {
  const tabs = await chrome.tabs.query({});
  await Promise.all(tabs.map((tab) => blockTab(tab, policy)));
}
// Watch pages cannot be decided from their URL (no channel in it),
// so already-open watch tabs are not replaced by blockOpenTabs.
// Tell their content scripts to re-evaluate instead; tabs without
// one (or on other sites) reject, which is fine.
async function notifyYouTubeTabs() {
  if (typeof chrome.tabs.sendMessage !== 'function') return;
  const tabs = await chrome.tabs.query({});
  await Promise.all(
    tabs.map((tab) =>
      chrome.tabs
        .sendMessage(tab.id, { type: 'preelock_policy_changed' })
        .catch(() => {}),
    ),
  );
}
chrome.tabs.onUpdated.addListener((id, change, tab) => {
  if (
    !change.url &&
    change.status !== 'loading' &&
    change.status !== 'complete'
  )
    return;
  void enqueue(async () => {
    const { policy } = await chrome.storage.session.get('policy');
    if (leaseValid(policy))
      await blockTab(
        { ...tab, id, pendingUrl: change.url || tab.pendingUrl },
        policy,
      );
  });
});
// A tab that navigates away from a channel-rule block stops being a
// channel-blocked tab (navigating to the block page itself keeps it).
// Registered after the replacement listener above so that listener
// keeps its position for the worker tests.
chrome.tabs.onUpdated.addListener((tabId, change) => {
  if (!change.url) return;
  void enqueue(async () => {
    if (change.url.startsWith(chrome.runtime.getURL('blocked.html'))) return;
    let url;
    try {
      url = new URL(change.url);
    } catch {
      return;
    }
    if (!/^https?:$/.test(url.protocol)) return;
    const { policy } = await chrome.storage.session.get('policy');
    if (!leaseValid(policy) || !channelBlockReason(change.url, policy))
      await clearChannelBlocked(tabId);
  });
});
if (chrome.tabs.onRemoved)
  chrome.tabs.onRemoved.addListener((tabId) => {
    void clearChannelBlocked(tabId);
  });
// Send only a hostname, only with desktop session consent. Never transmit URLs/titles.
async function activity(policy) {
  if (!policy?.observe_activity || !leaseValid(policy)) return {};
  try {
    const window = await chrome.windows.getLastFocused();
    if (!window.focused) return { browser_focused: false };
    const [tab] = await chrome.tabs.query({
      active: true,
      windowId: window.id,
    });
    const url = new URL(tab?.url || '');
    // Keyword rules cannot be judged from a hostname: report a matching
    // search page so the desktop engine counts it as distracted, even
    // when the search engine's domain itself is allowed. Only this
    // boolean travels to the app — never the URL or the query.
    // Exception: if the search page itself matches an exception rule
    // (e.g. the search engine's host is excepted), keyword blocks do
    // not apply and search_distracted is NOT reported — mirroring
    // urlBlockedByPolicy's precedence (exceptions before keywords).
    const found = searchPageQuery(tab?.url || '');
    const excepted =
      policy.mode === 'blocklist' &&
      (policy.exceptions || []).some((d) =>
        ruleMatches(d, url.hostname, url.pathname),
      );
    // Channel-rule blocks, mirroring search_distracted: an excepted
    // page never reports one. Two sources: a tab the worker sent to
    // the block page for a channel rule (its URL carries no channel,
    // so the flag map is the only record), and a tab still showing a
    // DNR-blocked channel/surface URL (its error page keeps the URL,
    // so the reason recomputes from it). The flag is also how a
    // content-script block on the block page itself is reported.
    const { channel_blocked_tabs = {} } = await chrome.storage.session.get(
      'channel_blocked_tabs',
    );
    const channel_blocked = Boolean(
      !excepted &&
      ((tab?.id != null && channel_blocked_tabs[tab.id]) ||
        channelBlockReason(tab?.url || '', policy)),
    );
    return {
      browser_focused: true,
      active_domain:
        /^https?:$/.test(url.protocol) && /^[a-z0-9.-]+$/i.test(url.hostname)
          ? url.hostname
          : '',
      search_distracted: Boolean(
        !excepted &&
        found &&
        searchQueryBlocked(policy.keywords || [], found.query),
      ),
      channel_blocked,
    };
  } catch {
    return { browser_focused: false };
  }
}
// Tell the bridge this browser disconnected on purpose, so the app stops
// showing it as connected right away instead of waiting for the
// heartbeat window to expire.
async function goodbye() {
  const { token, client } = await chrome.storage.local.get(['token', 'client']);
  if (!token || !client) return;
  try {
    await fetch('http://127.0.0.1:47832/sync', {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${token}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        client,
        browser: await browserIdentity(),
        applied_id: '',
        disconnect: true,
        v: EXTENSION_VERSION,
      }),
      signal: AbortSignal.timeout(4000),
    });
  } catch {}
}
async function sync() {
  const { token, client, disabled } = await chrome.storage.local.get([
    'token',
    'client',
    'disabled',
  ]);
  if (!token || disabled) {
    await removeRules();
    return;
  }
  const browser = await browserIdentity();
  let {
    policy,
    applied_id = '',
    blocks = 0,
  } = await chrome.storage.session.get(['policy', 'applied_id', 'blocks']);
  if (!leaseValid(policy)) {
    await removeRules();
    applied_id = '';
  }
  try {
    const response = await fetch('http://127.0.0.1:47832/sync', {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${token}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        client,
        browser,
        applied_id,
        blocks,
        error: '',
        v: EXTENSION_VERSION,
        ...(await activity(policy)),
      }),
      signal: AbortSignal.timeout(4000),
    });
    if (!response.ok)
      throw new Error(`Bridge rejected pairing (${response.status})`);
    const payload = await response.json();
    const next = payload.policy;
    let testResult = {};
    if (payload.test_id && !leaseValid(next)) {
      try {
        await selfTest();
        testResult = {
          test_id: payload.test_id,
          test_result: 'Passed · 8 browser checks',
        };
      } catch (error) {
        testResult = {
          test_id: payload.test_id,
          test_result: String(error).slice(0, 200),
        };
      }
    }
    if (!leaseValid(next)) {
      await removeRules();
    } else {
      const rules = await chrome.declarativeNetRequest.getSessionRules();
      const desired = compileRules(next);
      const matches =
        rules.length === desired.length &&
        desired.every((expected) => {
          const actual = rules.find((rule) => rule.id === expected.id);
          return (
            actual &&
            actual.priority === expected.priority &&
            actual.action.type === expected.action.type &&
            actual.condition.regexFilter === expected.condition.regexFilter &&
            JSON.stringify(actual.condition.requestDomains ?? []) ===
              JSON.stringify(expected.condition.requestDomains ?? []) &&
            JSON.stringify(
              [...(actual.condition.resourceTypes ?? [])].sort(),
            ) ===
              JSON.stringify([...expected.condition.resourceTypes].sort()) &&
            !actual.condition.tabIds &&
            !actual.condition.excludedTabIds
          );
        });
      if (next.id !== applied_id || !matches) {
        await chrome.declarativeNetRequest.updateSessionRules({
          removeRuleIds: rules.map((r) => r.id),
          addRules: desired,
        });
        blocks = 0;
      }
      await chrome.storage.session.set({
        policy: next,
        applied_id: next.id,
        blocks,
      });
      await blockOpenTabs(next);
      // Already-open watch tabs keep their video (their URLs carry
      // no channel); their content scripts re-evaluate on this.
      await notifyYouTubeTabs();
      await chrome.alarms.create('expiry', { when: next.expires_at });
    }
    await chrome.storage.session.set({ error: '', last_sync: Date.now() });
    // Acknowledge installation only after the browser accepted the rules.
    const installed = await chrome.storage.session.get([
      'applied_id',
      'blocks',
    ]);
    const acknowledgement = await fetch('http://127.0.0.1:47832/sync', {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${token}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        client,
        browser,
        applied_id: installed.applied_id || '',
        blocks: installed.blocks || 0,
        error: '',
        ...(await activity(next)),
        ...testResult,
      }),
      signal: AbortSignal.timeout(4000),
    });
    if (!acknowledgement.ok)
      throw new Error(`Bridge rejected pairing (${acknowledgement.status})`);
  } catch (error) {
    // Fail open rather than strand network access after a crash or broken installation.
    await removeRules();
    const detail = String(error);
    const message = detail.includes('(403)')
      ? 'Connection code rejected. Copy it again from Preelock Settings.'
      : detail.includes('(409)')
        ? 'Another browser profile is connected. Reset the connection in Preelock Settings.'
        : /fetch|timeout|abort|network/i.test(detail)
          ? 'Cannot reach Preelock. Open the desktop app, then reconnect.'
          : detail.slice(0, 200);
    await chrome.storage.session.set({ error: message });
  }
}
chrome.alarms.onAlarm.addListener(() => {
  void requestSync();
});
// Reload open YouTube tabs so the youtube.js content script is injected.
// Content scripts do not inject into tabs that were already open when the
// extension was installed/reloaded; without a full page load, watch-page
// channel blocking never runs on those tabs (SPA navigation doesn't help).
async function reloadYouTubeTabs() {
  try {
    const tabs = await chrome.tabs.query({
      url: ['*://youtube.com/*', '*://*.youtube.com/*'],
    });
    await Promise.all(
      tabs.map((tab) =>
        tab.id != null
          ? chrome.tabs.reload(tab.id).catch(() => {})
          : Promise.resolve(),
      ),
    );
  } catch {}
}
chrome.runtime.onInstalled.addListener(() => {
  void chrome.alarms.create('sync', { periodInMinutes: 0.5 });
  void requestSync();
  void reloadYouTubeTabs();
});
chrome.runtime.onStartup.addListener(() => {
  void chrome.alarms.create('sync', { periodInMinutes: 0.5 });
  void requestSync();
  void reloadYouTubeTabs();
});
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (sender.id !== chrome.runtime.id) return false;
  // YouTube content script (youtube.js): the worker owns the policy
  // and the decision, so the content script only reads the channel
  // from the page DOM and asks here. No URL or query ever leaves the
  // browser except the booleans already in the heartbeat.
  if (message.type === 'youtube_policy') {
    enqueue(async () => {
      const { policy } = await chrome.storage.session.get('policy');
      sendResponse({ policy: leaseValid(policy) ? policy : null });
    });
    return true;
  }
  if (message.type === 'youtube_check') {
    enqueue(async () => {
      const { policy } = await chrome.storage.session.get('policy');
      sendResponse(
        youtubeWatchDecision(
          message.url,
          message.channel ?? null,
          leaseValid(policy) ? policy : null,
          Boolean(message.unidentified),
        ),
      );
    });
    return true;
  }
  if (message.type === 'youtube_block') {
    enqueue(async () => {
      if (sender.tab?.id != null) {
        await markChannelBlocked(sender.tab.id);
        const { policy } = await chrome.storage.session.get('policy');
        await chrome.tabs
          .update(sender.tab.id, {
            url: blockedPageUrl(
              sender.tab.url,
              leaseValid(policy) ? policy : null,
            ),
          })
          .catch(() => {});
      }
      sendResponse({ ok: true });
    });
    return true;
  }
  if (message.type === 'self_test') {
    enqueue(selfTest).then(
      (result) => sendResponse({ result }),
      (e) => sendResponse({ error: String(e) }),
    );
    return true;
  }
  if (message.type === 'sync') {
    requestSync().then(
      () => sendResponse({ ok: true }),
      (e) => sendResponse({ error: String(e) }),
    );
    return true;
  }
  if (message.type === 'disconnect') {
    goodbye().then(
      () => sendResponse({ ok: true }),
      (e) => sendResponse({ error: String(e) }),
    );
    return true;
  }
  return false;
});
// Debug feedback is available for unpacked installations only; store counts, never URLs.
if (chrome.declarativeNetRequest.onRuleMatchedDebug)
  chrome.declarativeNetRequest.onRuleMatchedDebug.addListener(() => {
    void enqueue(async () => {
      const { blocks = 0 } = await chrome.storage.session.get('blocks');
      await chrome.storage.session.set({ blocks: blocks + 1 });
    });
  });
// While awake, fast polling reduces activation latency. Alarms are the fallback.
setInterval(() => {
  void requestSync();
}, 2000);
void chrome.alarms.create('sync', { periodInMinutes: 0.5 });
void requestSync();
