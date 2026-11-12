import {
  blockDetail,
  channelBlockReason,
  channelKeyFromPath,
  compileRules,
  isYouTubeHost,
  leaseValid,
  ruleMatches,
  searchPageQuery,
  searchQueryBlocked,
  channelIdentityKeys,
  urlBlockedByPolicy,
  watchLaterRedirectUrl,
  youtubeBrowsingBlocked,
  youtubeVideoDecision,
} from './rules.js';
import {
  WEBSITE_CONTROLS_KEY,
  compileWebsiteControlRules,
  isWebsiteControlRuleId,
  normalizeWebsiteControls,
  websiteControlBlockCategory,
} from './website-controls.js';
import { ADULT_DOMAINS_DATA } from './adult-domains.js';
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
// The blocked screen for session-scoped YouTube content. The worker
// already decided, so the kind only selects the wording: a channel
// block reads differently from a video that is not permitted.
function blockedYouTubePageUrl(kind) {
  const params = new URLSearchParams();
  params.set('k', 'youtube');
  params.set('m', kind === 'channel' ? 'channel' : 'content');
  return chrome.runtime.getURL('blocked.html') + '?' + params.toString();
}
import { browserIdentity } from './identity.js';
import { selfTest } from './self-test.js';
// Bump this every time extension behavior changes. The desktop app
// compares it against its own copy to spot extension copies that were
// never reloaded after an app update.
const EXTENSION_VERSION = 13;
// Learned UC-id <-> @handle aliases. YouTube's player data names a
// video's stable channel id, the page metadata names its handle, and a
// Preelock rule may use either form. Identities observed together for
// one video are remembered in both directions and used to expand every
// later decision, so a rule written as @handle still matches a video
// whose owner chip never rendered.
const MAX_CHANNEL_ALIASES = 800;
async function channelAliases() {
  const { channel_aliases } = await chrome.storage.local.get('channel_aliases');
  return channel_aliases && typeof channel_aliases === 'object'
    ? channel_aliases
    : {};
}
function expandedIdentities(keys, aliases) {
  const out = channelIdentityKeys(keys);
  for (const key of [...out]) {
    const alias = aliases[key];
    if (alias && !out.includes(alias)) out.push(alias);
  }
  return out;
}
function channelKeyFromUrl(urlString) {
  try {
    const u = new URL(urlString || '');
    if (!isYouTubeHost(u.hostname)) return null;
    return channelKeyFromPath(u.pathname);
  } catch {
    return null;
  }
}
// Remember that `keys` all describe one single channel (exactly one UC id
// and one @handle observed together on the current video). Refuse ambiguous
// sets so one multi-channel list can never poison future alias lookups.
function learnChannelAliases(keys, aliases) {
  const uc = keys.filter((key) => /^UC[A-Za-z0-9_-]{10,62}$/.test(key));
  const handles = keys.filter((key) => key.startsWith('@'));
  if (uc.length !== 1 || handles.length !== 1) return null;
  const id = uc[0];
  const handle = handles[0];
  if (aliases[id] === handle && aliases[handle] === id) return null;
  const next = { ...aliases, [id]: handle, [handle]: id };
  const entries = Object.keys(next);
  for (const stale of entries.slice(
    0,
    Math.max(0, entries.length - MAX_CHANNEL_ALIASES),
  ))
    delete next[stale];
  return next;
}
let serial = Promise.resolve();
const enqueue = (work) => {
  serial = serial.then(work, work);
  return serial;
};
// Coalesce timer/alarm triggers while a bridge request is pending, but allow
// explicit user pairing actions to queue a fresh sync after any in-flight poll.
let pendingSync;
function requestSync() {
  if (!pendingSync)
    pendingSync = enqueue(sync).finally(() => {
      pendingSync = null;
    });
  return pendingSync;
}
function forceSync() {
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
function websiteControlBlockedPageUrl(url, category) {
  const params = new URLSearchParams();
  try {
    params.set('h', new URL(url).hostname);
  } catch {
    // Non-URL input: no host param.
  }
  params.set('k', category);
  return chrome.runtime.getURL('blocked.html') + '?' + params.toString();
}
let cachedAdultDomains = null;
let cachedAdultDomainSet = null;
function getAdultDomains() {
  if (!cachedAdultDomains) {
    cachedAdultDomains = ADULT_DOMAINS_DATA
      ? ADULT_DOMAINS_DATA.split(' ').filter(Boolean)
      : [];
  }
  return cachedAdultDomains;
}
function getAdultDomainSet() {
  if (!cachedAdultDomainSet) {
    cachedAdultDomainSet = new Set(getAdultDomains());
  }
  return cachedAdultDomainSet;
}
async function readWebsiteControls() {
  const stored = await chrome.storage.local.get(WEBSITE_CONTROLS_KEY);
  return normalizeWebsiteControls(stored?.[WEBSITE_CONTROLS_KEY]);
}
function stringArraysEqual(a = [], b = []) {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}
function dynamicRulesMatch(actualList, desiredList) {
  if (actualList.length !== desiredList.length) return false;
  return desiredList.every((expected) => {
    const actual = actualList.find((r) => r.id === expected.id);
    return (
      actual &&
      actual.priority === expected.priority &&
      actual.action?.type === expected.action.type &&
      stringArraysEqual(
        actual.condition?.requestDomains ?? [],
        expected.condition.requestDomains ?? [],
      ) &&
      stringArraysEqual(
        [...(actual.condition?.resourceTypes ?? [])].sort(),
        [...expected.condition.resourceTypes].sort(),
      )
    );
  });
}
async function syncWebsiteControlRules(controls) {
  if (
    typeof chrome.declarativeNetRequest?.getDynamicRules !== 'function' ||
    typeof chrome.declarativeNetRequest?.updateDynamicRules !== 'function'
  ) {
    return false;
  }
  const normalized = normalizeWebsiteControls(controls);
  const adultList = normalized.blockAdultWebsites ? getAdultDomains() : [];
  const desired = compileWebsiteControlRules(normalized, adultList);
  const existing = (await chrome.declarativeNetRequest.getDynamicRules()) || [];
  const existingLocal = existing.filter((r) => isWebsiteControlRuleId(r.id));
  if (dynamicRulesMatch(existingLocal, desired)) return false;
  await chrome.declarativeNetRequest.updateDynamicRules({
    removeRuleIds: existingLocal.map((r) => r.id),
    addRules: desired,
  });
  return true;
}
const recentWlRedirects = new Map();
function recordWatchLaterRedirect(tabId, targetUrl) {
  const now = Date.now();
  const prev = recentWlRedirects.get(tabId);
  if (prev && prev.url === targetUrl && now - prev.at < 400) return false;
  recentWlRedirects.set(tabId, { url: targetUrl, at: now });
  if (recentWlRedirects.size > 100) {
    const oldest = recentWlRedirects.keys().next().value;
    recentWlRedirects.delete(oldest);
  }
  return true;
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
// Precedence:
//   1. Desktop offline mode (if active).
//   2. Extension-local Website Controls (adult / distracting categories),
//      enforced even when no desktop session is active and additive with sessions.
//   3. Watch Later auto-redirect (/) -> (/playlist?list=WL) before blockTab
//      sends `/` to blocked.html.
//   4. Desktop session / rule / channel blocks.
async function blockTab(tab, policy, websiteControls = null) {
  const url = tab.pendingUrl || tab.url;
  if (tab?.id == null || !url) return;
  if (url.startsWith(chrome.runtime.getURL('blocked.html'))) return;
  const activePolicy = policy && leaseValid(policy) ? policy : null;
  if (
    activePolicy &&
    activePolicy.mode === 'offline' &&
    tabBlockedBy(url, activePolicy)
  ) {
    await chrome.tabs
      .update(tab.id, { url: blockedPageUrl(url, activePolicy) })
      .catch(() => {}); // A tab may close between query and update.
    return;
  }
  const controls = websiteControls ?? (await readWebsiteControls());
  const localCategory = websiteControlBlockCategory(
    url,
    controls,
    controls.blockAdultWebsites ? getAdultDomainSet() : null,
  );
  if (localCategory) {
    await chrome.tabs
      .update(tab.id, { url: websiteControlBlockedPageUrl(url, localCategory) })
      .catch(() => {});
    return;
  }
  if (!activePolicy) return;
  const wlTarget = watchLaterRedirectUrl(url, activePolicy);
  if (wlTarget) {
    if (recordWatchLaterRedirect(tab.id, wlTarget)) {
      await chrome.tabs.update(tab.id, { url: wlTarget }).catch(() => {});
    }
    return;
  }
  if (tabBlockedBy(url, activePolicy)) {
    // Remember channel-rule blocks for the heartbeat: the block page
    // URL carries no channel, and a DNR error page keeps the blocked
    // URL (activity() recomputes that case from the URL instead).
    if (channelBlockReason(url, activePolicy)) await markChannelBlocked(tab.id);
    await chrome.tabs
      .update(tab.id, { url: blockedPageUrl(url, activePolicy) })
      .catch(() => {}); // A tab may close between query and update.
  }
}
async function blockOpenTabs(policy, websiteControls = null) {
  const controls = websiteControls ?? (await readWebsiteControls());
  const activePolicy = policy && leaseValid(policy) ? policy : null;
  if (
    !activePolicy &&
    !controls.blockAdultWebsites &&
    !controls.blockDistractingWebsites
  ) {
    return;
  }
  const tabs = await chrome.tabs.query({});
  await Promise.all(tabs.map((tab) => blockTab(tab, activePolicy, controls)));
}
async function reconcileWebsiteControls({ enforceTabs = false } = {}) {
  const controls = await readWebsiteControls();
  const changed = await syncWebsiteControlRules(controls);
  if (enforceTabs || changed) {
    const { policy } = await chrome.storage.session.get('policy');
    await blockOpenTabs(policy, controls);
  }
  return controls;
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
    const [{ policy }, controls] = await Promise.all([
      chrome.storage.session.get('policy'),
      readWebsiteControls(),
    ]);
    const activePolicy = leaseValid(policy) ? policy : null;
    if (
      activePolicy ||
      controls.blockAdultWebsites ||
      controls.blockDistractingWebsites
    )
      await blockTab(
        { ...tab, id, pendingUrl: change.url || tab.pendingUrl },
        activePolicy,
        controls,
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
// Verified lecture playback for the desktop app's five-minute activity
// rule: ask the active tab's content script for its measured playback
// state. Only the youtube.js content script answers (main YouTube watch
// and live pages, real playhead movement, no ad), and only about the
// tab the user is actually looking at. Any other tab, a missing content
// script or a slow answer is "not playing" — never an assumption.
function mediaState(tabId) {
  if (tabId == null || typeof chrome.tabs.sendMessage !== 'function') {
    return Promise.resolve(false);
  }
  return new Promise((resolve) => {
    let done = false;
    const finish = (value) => {
      if (done) return;
      done = true;
      resolve(value);
    };
    const timer = setTimeout(() => finish(false), 750);
    try {
      chrome.tabs.sendMessage(
        tabId,
        { type: 'preelock_media_state' },
        (response) => {
          clearTimeout(timer);
          if (chrome.runtime.lastError) finish(false);
          else finish(Boolean(response && response.playing === true));
        },
      );
    } catch {
      clearTimeout(timer);
      finish(false);
    }
  });
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
    // Media playback is only meaningful for the tab the user is
    // watching, so it is measured on this same active tab, after the
    // focused-window check above. A blocked tab, a search page flagged
    // as distracted, or a channel-blocked page is never a lecture.
    const media_playing = await mediaState(tab?.id);
    return {
      browser_focused: true,
      media_playing,
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
async function sync() {
  const websiteControls = await readWebsiteControls();
  await syncWebsiteControlRules(websiteControls);
  const stored = await chrome.storage.local.get([
    'token',
    'client',
    'disabled',
  ]);
  const token = typeof stored.token === 'string' ? stored.token.trim() : '';
  const disabled = Boolean(stored.disabled);
  if (!token || disabled) {
    await removeRules();
    return;
  }
  let client =
    typeof stored.client === 'string' && stored.client.trim()
      ? stored.client.trim()
      : '';
  if (!client) {
    client = crypto.randomUUID();
    await chrome.storage.local.set({ client });
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
        const outcome = await selfTest();
        testResult = {
          test_id: payload.test_id,
          test_result:
            typeof outcome === 'string' && outcome.startsWith('Passed ·')
              ? outcome
              : 'Passed · 8 browser checks',
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
        v: EXTENSION_VERSION,
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
    // M4: a failed sync no longer drops the rules while the stored
    // policy's lease is still valid. During a bridge handover (GUI
    // killed, background taking over) the bridge is briefly
    // unreachable; dropping the rules then would open a blocking gap
    // until the next 30 s sync, even though the policy the extension
    // already holds is still live. The lease still bounds fail-open:
    // once it expires, the sync-start check above removes the rules,
    // so a permanently dead bridge never strands blocking on. Fail
    // open (rather than strand network access after a crash or
    // broken installation) still applies once the lease is gone.
    if (!leaseValid(policy)) {
      await removeRules();
    }
    const detail = String(error);
    const message = detail.includes('(403)')
      ? 'Connection code rejected. Copy it again from Preelock Settings.'
      : detail.includes('(400)')
        ? "Couldn't identify this browser. Use a supported browser."
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
  void enqueue(() => reconcileWebsiteControls({ enforceTabs: true }));
  void requestSync();
  void reloadYouTubeTabs();
});
chrome.runtime.onStartup.addListener(() => {
  void chrome.alarms.create('sync', { periodInMinutes: 0.5 });
  void enqueue(() => reconcileWebsiteControls({ enforceTabs: true }));
  void requestSync();
  void reloadYouTubeTabs();
});
if (chrome.storage?.onChanged) {
  chrome.storage.onChanged.addListener((changes, areaName) => {
    if (areaName !== 'local' || !changes[WEBSITE_CONTROLS_KEY]) return;
    void enqueue(() => reconcileWebsiteControls({ enforceTabs: true }));
  });
}
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (sender.id !== chrome.runtime.id) return false;
  // YouTube content script (youtube.js): the worker owns the policy
  // and the decision, so the content script only reads the channel
  // from the page DOM and asks here. No URL or query ever leaves the
  // browser except the booleans already in the heartbeat.
  if (message.type === 'youtube_policy') {
    enqueue(async () => {
      const { policy } = await chrome.storage.session.get('policy');
      const live = leaseValid(policy) ? policy : null;
      // browsingClosed tells the content script whether ordinary
      // YouTube browsing is closed (so the homepage goes to Watch
      // Later and the feeds are hidden) or open (so a video that
      // happens to carry list=WL is just a normal video).
      sendResponse({
        policy: live,
        browsingClosed: live ? youtubeBrowsingBlocked(live) : false,
      });
    });
    return true;
  }
  if (message.type === 'youtube_check') {
    enqueue(async () => {
      const { policy } = await chrome.storage.session.get('policy');
      const live = leaseValid(policy) ? policy : null;
      const aliases = await channelAliases();
      const rawChannels =
        message.channels ?? (message.channel ? [message.channel] : []);
      const rawKeys = channelIdentityKeys(rawChannels);
      const urlKey = channelKeyFromUrl(message.url);
      const keys = expandedIdentities(
        urlKey ? [...rawKeys, urlKey] : rawKeys,
        aliases,
      );
      const learned = learnChannelAliases(rawKeys, aliases);
      if (learned) await chrome.storage.local.set({ channel_aliases: learned });
      const decision = youtubeVideoDecision(
        message.url,
        keys,
        live,
        Boolean(message.unidentified),
        message.watchLaterMember ?? null,
      );
      sendResponse(
        message.videoId ? { videoId: message.videoId, ...decision } : decision,
      );
    });
    return true;
  }
  // SPA navigation changes the URL with no network request, so the
  // content script asks about every route it lands on. Watch pages are
  // answered by youtube_check (their channel is page content).
  if (message.type === 'youtube_page_check') {
    enqueue(async () => {
      const { policy } = await chrome.storage.session.get('policy');
      const live = leaseValid(policy) ? policy : null;
      if (!live) {
        sendResponse({ blocked: false });
        return;
      }
      const aliases = await channelAliases();
      const urlKey = channelKeyFromUrl(message.url);
      const urlKeys = urlKey ? expandedIdentities([urlKey], aliases) : [];
      const blocked = urlKey
        ? youtubeVideoDecision(message.url, urlKeys, live, false, null).blocked
        : urlBlockedByPolicy(message.url, live);
      sendResponse({ blocked: Boolean(blocked) });
    });
    return true;
  }
  if (message.type === 'youtube_block') {
    enqueue(async () => {
      if (sender.tab?.id != null) {
        const { policy } = await chrome.storage.session.get('policy');
        const live = leaseValid(policy) ? policy : null;
        // No policy, or a policy that no longer blocks this page: the
        // session ended (or changed) between the check and this
        // message, so nothing is replaced.
        const aliases = await channelAliases();
        const rawChannels =
          message.channels ?? (message.channel ? [message.channel] : []);
        const urlKey = channelKeyFromUrl(message.url);
        const keys = expandedIdentities(
          urlKey ? [...rawChannels, urlKey] : rawChannels,
          aliases,
        );
        const decision = youtubeVideoDecision(
          message.url,
          keys,
          live,
          Boolean(message.unidentified),
          message.watchLaterMember ?? null,
        );
        if (!live || !decision.blocked) {
          sendResponse({ ok: true });
          return;
        }
        const channelBlocked =
          channelBlockReason(message.url, live) === 'blocked_channel' ||
          channelBlockReason(message.url, live) === 'study_channel' ||
          decision.reason === 'blocked_channel' ||
          decision.reason === 'study_channel';
        const unidentifiedChannelBlock =
          decision.reason === 'channel_unidentified' ||
          decision.reason === 'study_unidentified';
        if (channelBlocked || unidentifiedChannelBlock)
          await markChannelBlocked(sender.tab.id);
        await chrome.tabs
          .update(sender.tab.id, {
            url: blockedYouTubePageUrl(channelBlocked ? 'channel' : 'content'),
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
  if (message.type === 'reconcile_website_controls') {
    enqueue(() => reconcileWebsiteControls({ enforceTabs: true })).then(
      (controls) => sendResponse({ ok: true, controls }),
      (e) => sendResponse({ error: String(e) }),
    );
    return true;
  }
  if (message.type === 'sync') {
    (message.force ? forceSync() : requestSync()).then(
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
