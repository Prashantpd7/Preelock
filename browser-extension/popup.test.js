// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// The popup must not offer any way to disconnect the extension:
// no button, no label, and no disconnect message to the worker.
const here = import.meta.url.startsWith('file:')
  ? path.dirname(fileURLToPath(import.meta.url))
  : path.join(process.cwd(), 'browser-extension');
const html = readFileSync(path.join(here, 'popup.html'), 'utf8');
const js = readFileSync(path.join(here, 'popup.js'), 'utf8');
const css = readFileSync(path.join(here, 'popup.css'), 'utf8');

it('popup has no Disconnect button or disconnect action', () => {
  expect(html).not.toMatch(/disconnect/i);
  expect(js).not.toMatch(/disconnect/i);
});

it('popup header has compact connection badge without verbose desktop/browser status lines', () => {
  expect(html).toContain('id="connection-status"');
  expect(html).toContain('id="pair-view"');
  expect(html).toContain('id="website-controls"');
  expect(html).toContain('id="youtube-controls"');
  expect(html).not.toContain('id="browser-line"');
  expect(html).not.toContain('id="health"');
  expect(html).not.toContain('id="connected-view"');
  expect(js).not.toContain('Working — blocking sites');
  expect(js).not.toContain('Browser: ');
});

// The connection screen is compact: heading, instruction, code, Connect,
// error status. Browser identity is still detected silently in storage.
it('popup has no manual browser selection and preserves silent identity detection', () => {
  expect(html).not.toMatch(/Which browser is this/i);
  expect(html).not.toMatch(/browser-choices/i);
  expect(html).not.toMatch(/name="browser/i);
  expect(html).not.toMatch(/<fieldset/i);
  expect(html).not.toMatch(/type="radio"/i);
  expect(html).not.toContain('id="identify-view"');
  expect(html).toContain('id="token"');
  expect(html).toContain('Connect to Preelock');
  expect(html).toContain(
    'Paste your connection code from the Preelock desktop app.',
  );
  expect(js).toContain('detectBrowser');
  expect(js).toContain('browser_identity');
});

describe('Popup header, Website Controls, and YouTube Controls UI', () => {
  const controlsSource = readFileSync(
    path.join(here, 'youtube-controls.js'),
    'utf8',
  );
  let localStore;
  let sessionStore;
  let changeListeners;
  let sendMessageMock;
  let incognitoAllowed;

  function loadControlsApi() {
    const sandbox = { URL };
    sandbox.globalThis = sandbox;
    vm.createContext(sandbox);
    vm.runInContext(controlsSource, sandbox);
    return sandbox.__preelockYoutubeControls;
  }

  function pick(obj, keys) {
    if (keys == null) return { ...obj };
    const list = Array.isArray(keys) ? keys : [keys];
    const out = {};
    for (const k of list) {
      if (Object.hasOwn(obj, k)) out[k] = obj[k];
    }
    return out;
  }

  async function mountPopup() {
    vi.resetModules();
    document.documentElement.innerHTML = html;
    globalThis.__preelockYoutubeControls = loadControlsApi();
    vi.spyOn(globalThis, 'setInterval').mockReturnValue(0);

    Object.defineProperty(globalThis.navigator, 'userAgent', {
      configurable: true,
      value:
        'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36',
    });
    Object.defineProperty(globalThis.navigator, 'brave', {
      configurable: true,
      value: { isBrave: async () => true },
    });

    globalThis.chrome = {
      runtime: {
        lastError: null,
        sendMessage: sendMessageMock,
      },
      extension: {
        isAllowedIncognitoAccess: vi.fn(async () => incognitoAllowed),
      },
      storage: {
        local: {
          get: vi.fn(async (keys) => pick(localStore, keys)),
          set: vi.fn(async (items) => {
            const changes = {};
            for (const [k, v] of Object.entries(items)) {
              changes[k] = { oldValue: localStore[k], newValue: v };
              localStore[k] = v;
            }
            for (const fn of changeListeners) fn(changes, 'local');
          }),
          remove: vi.fn(async (keys) => {
            const list = Array.isArray(keys) ? keys : [keys];
            for (const k of list) delete localStore[k];
          }),
        },
        session: {
          get: vi.fn(async (keys) => pick(sessionStore, keys)),
        },
        onChanged: {
          addListener: vi.fn((fn) => {
            changeListeners.push(fn);
          }),
        },
      },
    };

    await import('./popup.js');
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
    await Promise.resolve();
  }

  beforeEach(() => {
    localStore = {
      token: 'a'.repeat(64),
      client: 'client-123',
      disabled: false,
    };
    sessionStore = {
      policy: {
        id: 'session-1',
        mode: 'blocklist',
        domains: [],
        expires_at: Date.now() + 60000,
      },
      last_sync: Date.now(),
    };
    changeListeners = [];
    incognitoAllowed = true;
    sendMessageMock = vi.fn(async () => ({ ok: true }));
  });

  afterEach(() => {
    vi.restoreAllMocks();
    delete globalThis.chrome;
    delete globalThis.__preelockYoutubeControls;
  });

  it('renders single-row Connected header above YouTube Controls and Website Controls without verbose lines', async () => {
    await mountPopup();

    const header = document.querySelector('.popup-header');
    const badge = document.getElementById('connection-status');
    const statusText = document.getElementById('connection-status-text');
    const pairView = document.getElementById('pair-view');
    const websiteSection = document.getElementById('website-controls');
    const controlsSection = document.getElementById('youtube-controls');

    expect(header.contains(badge)).toBe(true);
    expect(statusText.textContent.trim()).toBe('Connected');
    expect(badge.classList.contains('is-connected')).toBe(true);
    expect(pairView.hidden).toBe(true);
    expect(document.body.textContent).not.toContain('Working — blocking sites');
    expect(document.body.textContent).not.toMatch(/\bBrowser:\s/);
    expect(document.getElementById('incognito-warning').hidden).toBe(true);
    expect(localStore.browser_identity).toBe('com.brave.browser');

    // YouTube Controls appears cleanly ABOVE Website Controls.
    expect(
      Boolean(
        controlsSection.compareDocumentPosition(websiteSection) &
        Node.DOCUMENT_POSITION_FOLLOWING,
      ),
    ).toBe(true);

    const wsLabels = [...document.querySelectorAll('.ws-item-label')].map(
      (el) => el.textContent.trim(),
    );
    expect(wsLabels).toEqual([
      'Block Adult Websites',
      'Block Distracting Websites',
    ]);

    const groupTitles = [...document.querySelectorAll('.yt-group-toggle')].map(
      (btn) => btn.textContent.trim(),
    );
    expect(groupTitles).toEqual([
      'Home and Navigation',
      'Video Recommendations',
      'Video Player',
      'Social and Community',
      'Other YouTube Elements',
    ]);
  });

  it('shows Incognito warning only when disallowed in Incognito', async () => {
    incognitoAllowed = false;
    await mountPopup();
    const warn = document.getElementById('incognito-warning');
    expect(warn.hidden).toBe(false);
    expect(warn.textContent).toContain('Incognito');
  });

  it('persists Website Controls independently when connected and disconnected and notifies worker', async () => {
    localStore = { client: 'client-123' };
    sessionStore = {};
    await mountPopup();

    expect(
      document.getElementById('connection-status-text').textContent.trim(),
    ).toBe('Not connected');
    // The connect box stays hidden until the pill is clicked.
    expect(document.getElementById('pair-view').hidden).toBe(true);
    document.getElementById('connection-status').click();
    expect(document.getElementById('pair-view').hidden).toBe(false);

    const adultInput = document.querySelector(
      'input[data-website-control="blockAdultWebsites"]',
    );
    const distractInput = document.querySelector(
      'input[data-website-control="blockDistractingWebsites"]',
    );
    expect(adultInput.checked).toBe(false);
    expect(distractInput.checked).toBe(false);

    adultInput.checked = true;
    adultInput.dispatchEvent(new Event('change', { bubbles: true }));
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));

    expect(localStore.websiteControls).toEqual({
      blockAdultWebsites: true,
      blockDistractingWebsites: false,
    });
    expect(sendMessageMock).toHaveBeenCalledWith({
      type: 'reconcile_website_controls',
    });
    expect(document.getElementById('ws-active-count').textContent).toBe(
      '1 active',
    );

    // Rapid synchronous toggling of multiple switches must not lose state.
    adultInput.checked = false;
    adultInput.dispatchEvent(new Event('change', { bubbles: true }));
    adultInput.checked = true;
    adultInput.dispatchEvent(new Event('change', { bubbles: true }));
    distractInput.checked = true;
    distractInput.dispatchEvent(new Event('change', { bubbles: true }));
    await new Promise((r) => setTimeout(r, 10));
    expect(localStore.websiteControls).toEqual({
      blockAdultWebsites: true,
      blockDistractingWebsites: true,
    });
    expect(document.getElementById('ws-active-count').textContent).toBe(
      '2 active',
    );
    adultInput.checked = true;
    distractInput.checked = false;
    distractInput.dispatchEvent(new Event('change', { bubbles: true }));
    await new Promise((r) => setTimeout(r, 10));

    // Reopen while connected: state persists untouched.
    localStore.token = 'a'.repeat(64);
    await mountPopup();
    expect(
      document.querySelector('input[data-website-control="blockAdultWebsites"]')
        .checked,
    ).toBe(true);
    expect(
      document.querySelector(
        'input[data-website-control="blockDistractingWebsites"]',
      ).checked,
    ).toBe(false);
  });

  it('handles connection form validation, loading state, duplicate-submit guard, and connects cleanly', async () => {
    localStore = {};
    sessionStore = {};
    let resolveSync;
    sendMessageMock = vi.fn((msg) => {
      if (msg.type === 'sync') {
        return new Promise((r) => {
          resolveSync = r;
        });
      }
      return Promise.resolve({ ok: true });
    });

    await mountPopup();
    // The box only appears after the pill is clicked.
    expect(document.getElementById('pair-view').hidden).toBe(true);
    document.getElementById('connection-status').click();
    expect(document.getElementById('pair-view').hidden).toBe(false);
    const form = document.getElementById('pair');
    const tokenInput = document.getElementById('token');
    const connectBtn = document.getElementById('connect-btn');
    const pairStatus = document.getElementById('pair-status');

    // Invalid short code shows concise error without locking button.
    tokenInput.value = 'short';
    form.dispatchEvent(
      new Event('submit', { bubbles: true, cancelable: true }),
    );
    await Promise.resolve();
    expect(pairStatus.hidden).toBe(false);
    expect(pairStatus.textContent).toContain('64-character');
    expect(connectBtn.disabled).toBe(false);

    // Valid code enters loading state and guards duplicate submissions.
    tokenInput.value = 'b'.repeat(64);
    form.dispatchEvent(
      new Event('submit', { bubbles: true, cancelable: true }),
    );
    await new Promise((r) => setTimeout(r, 0));
    await Promise.resolve();
    expect(connectBtn.disabled).toBe(true);
    expect(connectBtn.textContent).toBe('Connecting…');

    form.dispatchEvent(
      new Event('submit', { bubbles: true, cancelable: true }),
    );
    await Promise.resolve();
    expect(
      sendMessageMock.mock.calls.filter(([m]) => m.type === 'sync').length,
    ).toBe(1);

    resolveSync({ ok: true });
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));

    expect(localStore.token).toBe('b'.repeat(64));
    expect(localStore.browser_identity).toBe('com.brave.browser');
    expect(connectBtn.disabled).toBe(false);
    expect(
      document.getElementById('connection-status-text').textContent.trim(),
    ).toBe('Connected');
    expect(document.getElementById('pair-view').hidden).toBe(true);
  });

  it('persists independent YouTube toggles, keeps synced toggles in sync, enforces Redirect/Hide Subscriptions mutual exclusion, restores on reload, and resets cleanly', async () => {
    await mountPopup();

    const checkControl = async (path, checked) => {
      const input = document.querySelector(
        `input[data-control-path="${path}"]`,
      );
      expect(input).not.toBeNull();
      input.checked = checked;
      input.dispatchEvent(new Event('change', { bubbles: true }));
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 0));
    };

    await checkControl('recommendations.hideRecommended', true);
    expect(localStore.youtubeControls.recommendations.hideVideoSidebar).toBe(
      false,
    );
    expect(localStore.youtubeControls.recommendations.hideRecommended).toBe(
      true,
    );

    const notifInputs = [
      ...document.querySelectorAll(
        'input[data-control-path="social.hideNotifications"]',
      ),
    ];
    expect(notifInputs.length).toBe(2);
    notifInputs[0].checked = true;
    notifInputs[0].dispatchEvent(new Event('change', { bubbles: true }));
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
    expect(notifInputs[1].checked).toBe(true);

    const chatInputs = [
      ...document.querySelectorAll(
        'input[data-control-path="recommendations.hideLiveChat"]',
      ),
    ];
    expect(chatInputs.length).toBe(2);
    chatInputs[1].checked = true;
    chatInputs[1].dispatchEvent(new Event('change', { bubbles: true }));
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
    expect(chatInputs[0].checked).toBe(true);

    // Redirect to Subscriptions and Hide Subscriptions are mutually
    // exclusive: enabling one turns the other off, and no warning
    // element ever renders.
    expect(document.querySelector('.yt-conflict-note')).toBeNull();
    const redirectInput = () =>
      document.querySelector(
        'input[data-control-path="home.redirectToSubscriptions"]',
      );
    const hideSubsInput = () =>
      document.querySelector(
        'input[data-control-path="navigation.hideSubscriptions"]',
      );
    await checkControl('home.redirectToSubscriptions', true);
    expect(redirectInput().checked).toBe(true);
    expect(hideSubsInput().checked).toBe(false);
    expect(localStore.youtubeControls.home.redirectToSubscriptions).toBe(true);
    expect(localStore.youtubeControls.navigation.hideSubscriptions).toBe(false);
    await checkControl('navigation.hideSubscriptions', true);
    expect(hideSubsInput().checked).toBe(true);
    expect(redirectInput().checked).toBe(false);
    expect(localStore.youtubeControls.home.redirectToSubscriptions).toBe(false);
    expect(localStore.youtubeControls.navigation.hideSubscriptions).toBe(true);
    expect(document.querySelector('.yt-conflict-note')).toBeNull();

    expect(document.getElementById('yt-active-count').textContent).toBe(
      '4 active',
    );
    expect(document.getElementById('yt-reset').hidden).toBe(false);

    expect(localStore.token).toBe('a'.repeat(64));
    expect(localStore.client).toBe('client-123');

    await mountPopup();
    expect(
      document.querySelector(
        'input[data-control-path="recommendations.hideRecommended"]',
      ).checked,
    ).toBe(true);
    expect(
      document.querySelector(
        'input[data-control-path="recommendations.hideVideoSidebar"]',
      ).checked,
    ).toBe(false);
    expect(
      document.querySelector(
        'input[data-control-path="home.redirectToSubscriptions"]',
      ).checked,
    ).toBe(false);
    expect(
      document.querySelector(
        'input[data-control-path="navigation.hideSubscriptions"]',
      ).checked,
    ).toBe(true);
    expect(document.querySelector('.yt-conflict-note')).toBeNull();
    expect(document.getElementById('yt-active-count').textContent).toBe(
      '4 active',
    );

    document.getElementById('yt-reset').click();
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
    expect(localStore.youtubeControls.home.redirectToSubscriptions).toBe(false);
    expect(localStore.youtubeControls.recommendations.hideRecommended).toBe(
      false,
    );
    expect(localStore.token).toBe('a'.repeat(64));
    expect(localStore.client).toBe('client-123');
    expect(document.getElementById('yt-reset').hidden).toBe(true);
    expect(document.getElementById('yt-active-count').hidden).toBe(true);

    await chrome.storage.local.set({
      youtubeControls: {
        home: { hideHomeFeed: true },
        player: { disableAutoplay: true },
      },
    });
    expect(
      document.querySelector('input[data-control-path="home.hideHomeFeed"]')
        .checked,
    ).toBe(true);
    expect(
      document.querySelector(
        'input[data-control-path="player.disableAutoplay"]',
      ).checked,
    ).toBe(true);
    expect(document.getElementById('yt-active-count').textContent).toBe(
      '2 active',
    );
  });

  it('never renders the redirect/hidden warning and keeps group layout unclipped with working scroll', async () => {
    expect(css).toContain('[hidden]');
    expect(css).toMatch(/\.yt-groups\s*\{[^}]*overflow-y:\s*auto/);
    expect(css).not.toContain('max-height: 330px');
    expect(css).toMatch(/\.yt-group\s*\{[^}]*flex:\s*0\s+0\s+auto/);
    expect(css).not.toMatch(/\.yt-group\s*\{[^}]*overflow:\s*hidden/);
    // The warning is gone entirely: no element, no CSS, no logic.
    expect(css).not.toContain('.yt-conflict-note');
    expect(js).not.toContain('subscriptionsConflictNote');
    expect(js).not.toContain('yt-conflict-note');

    await mountPopup();
    expect(document.querySelector('.yt-conflict-note')).toBeNull();
  });

  it('normalizes legacy stored state so Redirect and Hide Subscriptions are never both on', async () => {
    // Stored by an older version with both flags on.
    localStore.youtubeControls = {
      version: 1,
      home: { hideHomeFeed: false, redirectToSubscriptions: true },
      navigation: { hideExploreAndTrending: false, hideSubscriptions: true },
    };
    await mountPopup();

    // API-level: normalization keeps Redirect, drops Hide.
    const api = globalThis.__preelockYoutubeControls;
    const normalized = api.normalizeYoutubeControls({
      home: { redirectToSubscriptions: true },
      navigation: { hideSubscriptions: true },
    });
    expect(normalized.home.redirectToSubscriptions).toBe(true);
    expect(normalized.navigation.hideSubscriptions).toBe(false);

    // UI-level: only Redirect shows as on; no warning element.
    expect(
      document.querySelector(
        'input[data-control-path="home.redirectToSubscriptions"]',
      ).checked,
    ).toBe(true);
    expect(
      document.querySelector(
        'input[data-control-path="navigation.hideSubscriptions"]',
      ).checked,
    ).toBe(false);
    expect(document.querySelector('.yt-conflict-note')).toBeNull();
    expect(document.getElementById('yt-active-count').textContent).toBe(
      '1 active',
    );
    expect(
      document
        .querySelector('[data-group-id="home"] .yt-group-badge')
        .textContent.trim(),
    ).toBe('1');
  });

  it('keeps the connect box hidden until the Not connected pill is clicked, closes it with the X, and does nothing when Connected', async () => {
    localStore = { client: 'client-123' };
    sessionStore = {};
    await mountPopup();

    const badge = document.getElementById('connection-status');
    const pairView = document.getElementById('pair-view');
    expect(badge.tagName).toBe('BUTTON');
    expect(
      document.getElementById('connection-status-text').textContent.trim(),
    ).toBe('Not connected');
    expect(pairView.hidden).toBe(true);
    expect(document.getElementById('pair-close')).not.toBeNull();

    // Pill click reveals the box; X closes it; pill reopens it.
    badge.click();
    await Promise.resolve();
    expect(pairView.hidden).toBe(false);
    document.getElementById('pair-close').click();
    expect(pairView.hidden).toBe(true);
    badge.click();
    expect(pairView.hidden).toBe(false);

    // Reopen while connected: the box stays hidden and the Connected
    // pill does nothing.
    localStore.token = 'a'.repeat(64);
    await mountPopup();
    expect(
      document.getElementById('connection-status-text').textContent.trim(),
    ).toBe('Connected');
    expect(document.getElementById('pair-view').hidden).toBe(true);
    document.getElementById('connection-status').click();
    expect(document.getElementById('pair-view').hidden).toBe(true);
  });

  it('shows the stored connection error inside the connect box once opened', async () => {
    localStore = { client: 'client-123' };
    sessionStore = {
      error: 'Connection code rejected. Copy it again from Preelock Settings.',
    };
    await mountPopup();

    // Box is hidden until the pill is clicked; the error is not
    // visible yet.
    expect(document.getElementById('pair-view').hidden).toBe(true);
    document.getElementById('connection-status').click();
    expect(document.getElementById('pair-view').hidden).toBe(false);
    const pairStatus = document.getElementById('pair-status');
    expect(pairStatus.hidden).toBe(false);
    expect(pairStatus.textContent).toContain('Connection code rejected');
  });

  it('starts every group and nested sub-control dropdown closed on open and reopen even with active settings', async () => {
    localStore.youtubeControls = {
      version: 1,
      home: { hideHomeFeed: false, redirectToSubscriptions: true },
      navigation: { hideExploreAndTrending: false, hideSubscriptions: false },
      recommendations: {
        hideVideoSidebar: false,
        hideRecommended: true,
        hideLiveChat: false,
        hidePlaylist: false,
        hideFundraiser: false,
        hideEndScreenFeed: false,
        hideEndScreenCards: false,
        hideShorts: false,
        hideMixes: false,
      },
      player: { hideDescription: true },
      social: { hideNotifications: true },
    };

    await mountPopup();

    const groupToggles = [...document.querySelectorAll('.yt-group-toggle')];
    const groupBodies = [...document.querySelectorAll('.yt-group-body')];
    const subToggles = [...document.querySelectorAll('.yt-sub-toggle')];
    const subPanels = [...document.querySelectorAll('.yt-subcontrols')];

    expect(groupToggles.length).toBe(5);
    expect(
      groupToggles.every(
        (btn) => btn.getAttribute('aria-expanded') === 'false',
      ),
    ).toBe(true);
    expect(groupBodies.every((body) => body.hidden === true)).toBe(true);
    expect(subToggles.length).toBeGreaterThan(0);
    expect(
      subToggles.every((btn) => btn.getAttribute('aria-expanded') === 'false'),
    ).toBe(true);
    expect(subPanels.every((panel) => panel.hidden === true)).toBe(true);

    // Active badges still show which groups contain enabled controls.
    expect(
      document
        .querySelector('[data-group-id="home"] .yt-group-badge')
        .textContent.trim(),
    ).toBe('1');
    expect(
      document
        .querySelector('[data-group-id="recommendations"] .yt-group-badge')
        .textContent.trim(),
    ).toBe('1');

    // User can open dropdowns and see saved controls, then closing/reopening closes them again.
    groupToggles[1].click();
    subToggles[0].click();
    expect(groupToggles[1].getAttribute('aria-expanded')).toBe('true');
    expect(groupBodies[1].hidden).toBe(false);
    expect(subToggles[0].getAttribute('aria-expanded')).toBe('true');
    expect(subPanels[0].hidden).toBe(false);
    expect(
      document.querySelector(
        'input[data-control-path="recommendations.hideRecommended"]',
      ).checked,
    ).toBe(true);

    await mountPopup();
    expect(
      [...document.querySelectorAll('.yt-group-toggle')].every(
        (btn) => btn.getAttribute('aria-expanded') === 'false',
      ),
    ).toBe(true);
    expect(
      [...document.querySelectorAll('.yt-group-body')].every(
        (body) => body.hidden === true,
      ),
    ).toBe(true);
    expect(
      [...document.querySelectorAll('.yt-sub-toggle')].every(
        (btn) => btn.getAttribute('aria-expanded') === 'false',
      ),
    ).toBe(true);
    expect(
      [...document.querySelectorAll('.yt-subcontrols')].every(
        (panel) => panel.hidden === true,
      ),
    ).toBe(true);
  });

  it('requests a live worker sync on open when paired and flips to Not connected if the bridge rejects or is down', async () => {
    localStore = {
      token: 'a'.repeat(64),
      client: '11111111-1111-1111-1111-111111111111',
      disabled: false,
    };
    sessionStore = {};
    sendMessageMock = vi.fn(async (msg) => {
      if (msg.type === 'sync') {
        sessionStore.error =
          'Cannot reach Preelock. Open the desktop app, then reconnect.';
        return { ok: true };
      }
      return { ok: true };
    });

    await mountPopup();
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));

    expect(sendMessageMock).toHaveBeenCalledWith({ type: 'sync' });
    expect(
      document.getElementById('connection-status-text').textContent.trim(),
    ).toBe('Not connected');
  });
});
