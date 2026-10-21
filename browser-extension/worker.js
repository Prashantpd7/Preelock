import { compileRules, leaseValid } from './rules.js';
import { browserIdentity } from './identity.js';
import { selfTest } from './self-test.js';
// Bump this every time extension behavior changes. The desktop app
// compares it against its own copy to spot extension copies that were
// never reloaded after an app update.
const EXTENSION_VERSION = 3;
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
  await chrome.storage.session.set({ policy: null, applied_id: '' });
}
// True if the new policy would block this URL. Used to reload already-open
// tabs when a session starts, so a page that was open beforehand (e.g.
// YouTube) is stopped instead of silently continuing.
function tabBlockedBy(url, policy) {
  let hostname = '';
  try {
    const parsed = new URL(url || '');
    if (!/^https?:$/.test(parsed.protocol)) return false;
    hostname = parsed.hostname.toLowerCase();
  } catch {
    return false;
  }
  if (!hostname) return false;
  if (['localhost', '127.0.0.1', '[::1]'].includes(hostname)) return false;
  if (policy.mode === 'offline') return true;
  const domains = (policy.domains || []).map((d) => String(d).toLowerCase());
  const listed = domains.some(
    (d) => hostname === d || hostname.endsWith('.' + d),
  );
  if (policy.mode === 'blocklist') return listed;
  if (policy.mode === 'allowlist') return !listed;
  return false;
}
// DNR cannot block a page served entirely from a website service worker.
// Replace blocked top-level tabs too, without clearing browsing data or
// storing their URLs. This also stops already-playing cached video.
async function blockTab(tab, policy) {
  if (tab?.id != null && tabBlockedBy(tab.pendingUrl || tab.url, policy)) {
    await chrome.tabs
      .update(tab.id, {
        url: chrome.runtime.getURL('blocked.html'),
      })
      .catch(() => {}); // A tab may close between query and update.
  }
}
async function blockOpenTabs(policy) {
  const tabs = await chrome.tabs.query({});
  await Promise.all(tabs.map((tab) => blockTab(tab, policy)));
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
    return {
      browser_focused: true,
      active_domain:
        /^https?:$/.test(url.protocol) && /^[a-z0-9.-]+$/i.test(url.hostname)
          ? url.hostname
          : '',
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
      ? 'Connection code rejected. Copy it again from VYSTRIQ Settings.'
      : detail.includes('(409)')
        ? 'Another browser profile is connected. Reset the connection in VYSTRIQ Settings.'
        : /fetch|timeout|abort|network/i.test(detail)
          ? 'Cannot reach VYSTRIQ. Open the desktop app, then reconnect.'
          : detail.slice(0, 200);
    await chrome.storage.session.set({ error: message });
  }
}
chrome.alarms.onAlarm.addListener(() => {
  void requestSync();
});
chrome.runtime.onInstalled.addListener(() => {
  void chrome.alarms.create('sync', { periodInMinutes: 0.5 });
  void requestSync();
});
chrome.runtime.onStartup.addListener(() => {
  void chrome.alarms.create('sync', { periodInMinutes: 0.5 });
  void requestSync();
});
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (sender.id !== chrome.runtime.id) return false;
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
