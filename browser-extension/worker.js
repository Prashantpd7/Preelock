import { compileRules, leaseValid } from './rules.js';
import { selfTest } from './self-test.js';
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
  await chrome.action.setBadgeText({ text: '' });
}
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
        applied_id,
        blocks,
        error: '',
        ...(await activity(policy)),
      }),
      signal: AbortSignal.timeout(4000),
    });
    if (!response.ok)
      throw new Error(`Bridge rejected pairing (${response.status})`);
    const next = (await response.json()).policy;
    if (!leaseValid(next)) {
      await removeRules();
    } else {
      if (next.id !== applied_id) {
        const rules = await chrome.declarativeNetRequest.getSessionRules();
        await chrome.declarativeNetRequest.updateSessionRules({
          removeRuleIds: rules.map((r) => r.id),
          addRules: compileRules(next),
        });
        blocks = 0;
      }
      await chrome.storage.session.set({
        policy: next,
        applied_id: next.id,
        blocks,
      });
      await chrome.alarms.create('expiry', { when: next.expires_at });
      await chrome.action.setBadgeText({ text: 'ON' });
      await chrome.action.setBadgeBackgroundColor({ color: '#547d32' });
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
        applied_id: installed.applied_id || '',
        blocks: installed.blocks || 0,
        error: '',
        ...(await activity(next)),
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
  if (message.type === 'release' && message.confirmation === 'END FOCUS') {
    enqueue(async () => {
      await chrome.storage.local.set({ disabled: true });
      await removeRules();
      await chrome.storage.session.set({
        error:
          'Emergency browser release. Desktop will detect lost enforcement.',
      });
    }).then(() => sendResponse({ ok: true }));
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
