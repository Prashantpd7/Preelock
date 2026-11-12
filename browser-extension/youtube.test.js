import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
import { youtubeVideoDecision, youtubeWatchDecision } from './rules.js';
// youtube.js is a classic content script, not an ES module, so it
// cannot be imported: load its source in a vm context with stubbed
// browser globals and call the pure functions it exposes on
// globalThis.__preelockYoutube. This tests the actual shipped
// script, not a copy of its logic.
// Under vitest's transform import.meta.url is not a file URL, so
// fall back to the project root when it is not one.
const here = import.meta.url.startsWith('file:')
  ? path.dirname(fileURLToPath(import.meta.url))
  : path.join(process.cwd(), 'browser-extension');
const source = readFileSync(path.join(here, 'youtube.js'), 'utf8');
function loadScript(overrides = {}) {
  const messages = [];
  const replaced = [];
  const elements = {};
  const storage = new Map();
  const makeElement = (tag) => ({
    tagName: tag,
    id: '',
    textContent: '',
    setAttribute() {},
    appendChild() {},
    remove() {
      delete elements[this.id];
    },
  });
  const register = (el) => {
    elements[el.id] = el;
    return el;
  };
  const document = {
    visibilityState: 'visible',
    getElementById: (id) => elements[id] || null,
    createElement: (tag) => makeElement(tag),
    querySelector: () => overrides.player ?? null,
    querySelectorAll: (selector) => {
      if (typeof overrides.queryAll === 'function')
        return overrides.queryAll(selector);
      return overrides.ownerNodes ?? [];
    },
    head: { appendChild: register },
    body: { appendChild: register },
    documentElement: { appendChild: register },
  };
  const context = {
    console,
    // vm contexts do not inherit Node's URL global, but the content
    // script uses new URL(); browsers always provide it.
    URL,
    Date: { now: () => overrides.now ?? 0 },
    setTimeout: (fn, delay) => {
      // Run the observer debounce immediately, but leave IPC timeout
      // callbacks pending so the synchronous mock response can win.
      if (delay === 500) fn();
      return 0;
    },
    clearTimeout: () => {},
    setInterval: () => 0,
    MutationObserver: class {
      constructor() {}
      observe() {}
      disconnect() {}
    },
    location: {
      href: overrides.href || 'https://www.youtube.com/watch?v=x',
      replace: (url) => {
        replaced.push(url);
      },
    },
    document,
    // Membership evidence lives in the tab's own sessionStorage.
    sessionStorage: {
      getItem: (key) => (storage.has(key) ? storage.get(key) : null),
      setItem: (key, value) => storage.set(key, String(value)),
      removeItem: (key) => storage.delete(key),
    },
    chrome: {
      runtime: {
        id: 'test-extension',
        lastError: null,
        sendMessage: (message, callback) => {
          messages.push(message);
          const response = overrides.onMessage
            ? overrides.onMessage(message)
            : null;
          callback(response);
        },
        onMessage: { addListener: () => {} },
      },
    },
  };
  context.window = {
    location: context.location,
    addEventListener: () => {},
  };
  context.globalThis = context;
  vm.createContext(context);
  vm.runInContext(source, context);
  return {
    api: context.__preelockYoutube,
    messages,
    replaced,
    elements,
    storage,
  };
}
const settle = async () => {
  await Promise.resolve();
  await new Promise((resolve) => setTimeout(resolve, 0));
};
describe('youtube.js content script', () => {
  it('parses channel keys from paths and hrefs, mirroring rules.js', () => {
    const { api } = loadScript();
    expect(api.parseChannelKeyFromPath('/@Handle/videos')).toBe('@handle');
    expect(
      api.parseChannelKeyFromPath('/channel/UCabcdefghijklmnopqrstuv'),
    ).toBe('UCabcdefghijklmnopqrstuv');
    expect(api.parseChannelKeyFromPath('/c/SomeName')).toBe('c:somename');
    expect(api.parseChannelKeyFromPath('/user/SomeName')).toBe('user:somename');
    expect(api.parseChannelKeyFromPath('/watch')).toBe(null);
    expect(api.parseChannelKeyFromPath('/shorts/abc')).toBe(null);
    expect(api.parseChannelKeyFromHref('/@Owner')).toBe('@owner');
    expect(
      api.parseChannelKeyFromHref(
        'https://www.youtube.com/channel/UCabcdefghijklmnopqrstuv',
      ),
    ).toBe('UCabcdefghijklmnopqrstuv');
    expect(api.parseChannelKeyFromHref('https://example.com/@x')).toBe(null);
    expect(
      api.extractChannelKey([
        '/watch?v=x',
        'https://www.youtube.com/@Owner',
        '/channel/UCabcdefghijklmnopqrstuv',
      ]),
    ).toBe('@owner');
    expect(api.extractChannelKey(['/watch?v=x'])).toBe(null);
  });
  it('treats watch, shorts and live pages as video pages, not embeds', () => {
    const { api } = loadScript();
    expect(api.isVideoPage('/watch')).toBe(true);
    expect(api.isVideoPage('/shorts/abc')).toBe(true);
    expect(api.isVideoPage('/live/abc')).toBe(true);
    expect(api.isVideoPage('/shorts')).toBe(false);
    expect(api.isVideoPage('/embed/abc')).toBe(false);
    expect(api.isVideoPage('/@handle')).toBe(false);
    expect(api.isVideoPage('/')).toBe(false);
    expect(api.isYouTubeHost('www.youtube.com')).toBe(true);
    expect(api.isYouTubeHost('music.youtube.com')).toBe(true);
    expect(api.isYouTubeHost('youtube.com.evil.com')).toBe(false);
  });
  it('verifies a lecture only from measured, visible foreground playback', () => {
    const { api } = loadScript();
    const playing = {
      lecturePage: true,
      adShowing: false,
      paused: false,
      ended: false,
      currentTime: 120,
      duration: 1800,
      visible: true,
      playheadAdvanced: true,
    };
    expect(api.lecturePlaying(playing)).toBe(true);
    // Paused, stopped and merely loaded pages are not playback.
    expect(api.lecturePlaying({ ...playing, paused: true })).toBe(false);
    expect(api.lecturePlaying({ ...playing, ended: true })).toBe(false);
    expect(api.lecturePlaying({ ...playing, currentTime: 0 })).toBe(false);
    // A still playhead (a buffering or frozen player) is not playback.
    expect(api.lecturePlaying({ ...playing, playheadAdvanced: false })).toBe(
      false,
    );
    // Advertisements never qualify.
    expect(api.lecturePlaying({ ...playing, adShowing: true })).toBe(false);
    // A hidden tab is not something the user is watching.
    expect(api.lecturePlaying({ ...playing, visible: false })).toBe(false);
    // Shorts, embeds, channel pages, YouTube Music and other sites never
    // verify a lecture.
    expect(api.lecturePlaying({ ...playing, lecturePage: false })).toBe(false);
    expect(api.lecturePlaying({ ...playing, duration: 45 })).toBe(false);
    expect(api.lecturePlaying(null)).toBe(false);
    expect(api.isLectureHost('www.youtube.com')).toBe(true);
    expect(api.isLectureHost('m.youtube.com')).toBe(true);
    expect(api.isLectureHost('youtube.com')).toBe(true);
    expect(api.isLectureHost('music.youtube.com')).toBe(false);
    expect(api.isLectureHost('example.com')).toBe(false);
    expect(api.isLecturePath('/watch')).toBe(true);
    expect(api.isLecturePath('/live/abc')).toBe(true);
    expect(api.isLecturePath('/shorts/abc')).toBe(false);
    expect(api.isLecturePath('/embed/abc')).toBe(false);
    expect(api.isLecturePath('/@handle')).toBe(false);
    expect(api.isLecturePath('/')).toBe(false);
  });
  it('blocks a positively identified channel via the worker', async () => {
    const policy = {
      mode: 'blocklist',
      youtube_blocked_channels: ['@blocked'],
    };
    const { messages } = loadScript({
      ownerNodes: [{ getAttribute: () => '/@blocked' }],
      player: { pause: () => {} },
      onMessage: (message) => {
        if (message.type === 'youtube_policy') return { policy };
        if (message.type === 'youtube_check')
          return { blocked: message.channel === '@blocked' };
        return { ok: true };
      },
    });
    // The script evaluates on load; flush microtasks.
    await settle();
    expect(
      messages.some(
        (m) => m.type === 'youtube_check' && m.channel === '@blocked',
      ),
    ).toBe(true);
    expect(messages.some((m) => m.type === 'youtube_block')).toBe(true);
  });
  it('plays an unidentified channel in block mode (fail-open)', async () => {
    const policy = {
      mode: 'blocklist',
      youtube_blocked_channels: ['@blocked'],
    };
    const { messages } = loadScript({
      ownerNodes: [],
      player: { pause: () => {} },
      onMessage: (message) => {
        if (message.type === 'youtube_policy') return { policy };
        return { blocked: false };
      },
    });
    await settle();
    expect(messages.some((m) => m.type === 'youtube_block')).toBe(false);
  });
  it('reads video ids only from playlist listings', () => {
    const { api } = loadScript();
    expect(
      api.videoIdsFromHrefs([
        '/watch?v=aaaaaaaaaaa',
        'https://www.youtube.com/watch?v=aaaaaaaaaaa&list=WL&index=2',
        '/watch?v=bbbbbbbbbbb',
        '/@channel',
        '/watch',
        'https://example.com/watch?v=ccccccccccc',
      ]),
    ).toEqual(['aaaaaaaaaaa', 'bbbbbbbbbbb']);
    expect(api.videoIdFromUrl('https://www.youtube.com/watch?v=abc123')).toBe(
      'abc123',
    );
    expect(api.videoIdFromUrl('https://www.youtube.com/watch?list=WL')).toBe(
      null,
    );
  });
  it('accepts a Watch Later video only with playlist evidence', () => {
    const { api } = loadScript();
    expect(
      api.isWatchLaterVideoUrl(
        'https://www.youtube.com/watch?v=abc123&list=WL',
      ),
    ).toBe(true);
    expect(
      api.isWatchLaterVideoUrl(
        'https://m.youtube.com/watch?v=abc123&list=WL&index=3',
      ),
    ).toBe(true);
    // The playlist route, other ids, missing ids and other surfaces are
    // not the video route.
    for (const url of [
      'https://www.youtube.com/playlist?list=WL',
      'https://www.youtube.com/watch?v=abc123',
      'https://www.youtube.com/watch?list=WL',
      'https://www.youtube.com/watch?v=abc123&list=WLxyz',
      'https://music.youtube.com/watch?v=abc123&list=WL',
      'https://youtube.com.evil.com/watch?v=abc123&list=WL',
    ])
      expect(api.isWatchLaterVideoUrl(url)).toBe(false);
    expect(api.watchLaterMembership('abc123', ['abc123'])).toBe(true);
    // No evidence yet is not an allowance.
    expect(api.watchLaterMembership('abc123', [])).toBe(null);
    expect(api.watchLaterMembership('abc123', ['other'])).toBe(null);
    expect(api.watchLaterMembership(null, ['abc123'])).toBe(null);
  });
  it('never treats another playlist panel as Watch Later evidence', async () => {
    const policy = {
      mode: 'allowlist',
      domains: ['github.com'],
      youtube_allow_watch_later: true,
    };
    const { messages } = loadScript({
      href: 'https://www.youtube.com/watch?v=abc123&list=WL',
      player: { pause: () => {}, muted: false },
      // A Mix or other playlist panel lists the same video, but its
      // links carry a different list id: not Watch Later membership.
      queryAll: (selector) =>
        selector.includes('playlist-panel')
          ? [{ getAttribute: () => '/watch?v=abc123&list=RDabc123' }]
          : [],
      onMessage: (message) => {
        if (message.type === 'youtube_policy')
          return { policy, browsingClosed: true };
        if (message.type === 'youtube_page_check') return { blocked: false };
        if (message.type === 'youtube_check')
          return youtubeWatchDecision(
            message.url,
            message.channel ?? null,
            policy,
            Boolean(message.unidentified),
            message.watchLaterMember ?? null,
          );
        return { ok: true };
      },
    });
    await settle();
    const check = messages.find((m) => m.type === 'youtube_check');
    expect(check.watchLaterMember).toBe(null);
    expect(messages.some((m) => m.type === 'youtube_block')).toBe(false);
  });
  it('hides restricted surfaces and keeps the playlist panel', () => {
    const { api } = loadScript();
    expect(api.styleCss(null, false)).toBe('');
    const channelPolicy = {
      mode: 'blocklist',
      youtube_blocked_channels: ['@blocked'],
    };
    const selectors = api.hiddenSelectors(channelPolicy, false);
    expect(selectors).toContain('ytd-watch-next-secondary-results-renderer');
    expect(selectors).toContain('ytd-reel-shelf-renderer');
    // The Watch Later panel inside #secondary is never hidden.
    expect(selectors.some((s) => s.includes('playlist-panel'))).toBe(false);
    // Home-feed hiding belongs to policies that close YouTube browsing.
    expect(selectors.some((s) => s.includes('page-subtype="home"'))).toBe(
      false,
    );
    expect(
      api
        .hiddenSelectors(channelPolicy, true)
        .some((s) => s.includes('page-subtype="home"')),
    ).toBe(true);
    // The neutral guard is part of the same stylesheet.
    const css = api.styleCss(channelPolicy, false);
    expect(css).toMatch(/#preelock-yt-guard/);
    expect(css).toContain('display:none!important');
    // A policy without YouTube controls hides nothing.
    expect(
      api.styleCss({ mode: 'blocklist', domains: ['youtube.com'] }, false),
    ).toBe('');
  });
  it('allows a Watch Later video whose id the playlist rendered', async () => {
    const policy = {
      mode: 'allowlist',
      domains: ['github.com'],
      youtube_allow_watch_later: true,
    };
    const { messages, elements } = loadScript({
      href: 'https://www.youtube.com/watch?v=abc123&list=WL',
      player: { pause: () => {}, muted: false },
      queryAll: (selector) =>
        selector.includes('playlist-panel')
          ? [{ getAttribute: () => '/watch?v=abc123&list=WL&index=1' }]
          : [],
      onMessage: (message) => {
        if (message.type === 'youtube_policy')
          return { policy, browsingClosed: true };
        if (message.type === 'youtube_page_check') return { blocked: false };
        if (message.type === 'youtube_check')
          return youtubeWatchDecision(
            message.url,
            message.channel ?? null,
            policy,
            Boolean(message.unidentified),
            message.watchLaterMember ?? null,
          );
        return { ok: true };
      },
    });
    await settle();
    const check = messages.find((m) => m.type === 'youtube_check');
    expect(check).toBeDefined();
    expect(check.watchLaterMember).toBe(true);
    expect(messages.some((m) => m.type === 'youtube_block')).toBe(false);
    // The neutral cover is gone once the video is allowed.
    expect(elements['preelock-yt-guard']).toBeUndefined();
  });
  it('keeps the cover up while Watch Later membership is unverified', async () => {
    const policy = {
      mode: 'allowlist',
      domains: ['github.com'],
      youtube_allow_watch_later: true,
    };
    const { messages, elements } = loadScript({
      href: 'https://www.youtube.com/watch?v=abc123&list=WL',
      player: { pause: () => {}, muted: false },
      queryAll: () => [],
      onMessage: (message) => {
        if (message.type === 'youtube_policy')
          return { policy, browsingClosed: true };
        if (message.type === 'youtube_page_check') return { blocked: false };
        if (message.type === 'youtube_check')
          return youtubeWatchDecision(
            message.url,
            message.channel ?? null,
            policy,
            Boolean(message.unidentified),
            message.watchLaterMember ?? null,
          );
        return { ok: true };
      },
    });
    await settle();
    const check = messages.find((m) => m.type === 'youtube_check');
    expect(check.watchLaterMember).toBe(null);
    // Nothing is allowed and nothing is blocked yet: the guard stays.
    expect(messages.some((m) => m.type === 'youtube_block')).toBe(false);
    expect(elements['preelock-yt-guard']).toBeDefined();
  });
  it('blocks an SPA route the policy does not permit', async () => {
    const policy = {
      mode: 'allowlist',
      domains: ['github.com'],
      youtube_allow_watch_later: true,
    };
    const { messages } = loadScript({
      href: 'https://www.youtube.com/@somechannel',
      now: 1_700_000_000_000,
      onMessage: (message) => {
        if (message.type === 'youtube_policy')
          return { policy, browsingClosed: true };
        if (message.type === 'youtube_page_check') return { blocked: true };
        return { ok: true };
      },
    });
    await settle();
    // Every route is asked about, not only watch pages.
    expect(messages.some((m) => m.type === 'youtube_page_check')).toBe(true);
    expect(messages.some((m) => m.type === 'youtube_block')).toBe(true);
  });
  it('sends the homepage to Watch Later when it is the only content left', async () => {
    const policy = {
      mode: 'allowlist',
      domains: ['github.com'],
      youtube_allow_watch_later: true,
    };
    const { replaced, messages } = loadScript({
      href: 'https://www.youtube.com/',
      now: 1_700_000_000_000,
      onMessage: (message) => {
        if (message.type === 'youtube_policy')
          return { policy, browsingClosed: true };
        return { ok: true };
      },
    });
    await settle();
    expect(replaced).toEqual(['https://www.youtube.com/playlist?list=WL']);
    expect(messages.some((m) => m.type === 'youtube_block')).toBe(false);
  });
  it('leaves an ordinary YouTube tab alone when YouTube browsing is open', async () => {
    const policy = {
      mode: 'blocklist',
      domains: [],
      youtube_blocked_channels: ['@blocked'],
    };
    const { replaced, messages } = loadScript({
      href: 'https://www.youtube.com/',
      now: 1_700_000_000_000,
      onMessage: (message) => {
        if (message.type === 'youtube_policy')
          return { policy, browsingClosed: false };
        if (message.type === 'youtube_page_check') return { blocked: false };
        return { ok: true };
      },
    });
    await settle();
    expect(replaced).toEqual([]);
    expect(messages.some((m) => m.type === 'youtube_block')).toBe(false);
  });
});

describe('youtube.js Unhook controls, SPA navigation, and channel hardening', () => {
  const controlsSource = readFileSync(
    path.join(here, 'youtube-controls.js'),
    'utf8',
  );

  function loadIntegrated(overrides = {}) {
    const messages = [];
    const replaced = [];
    const postedMessages = [];
    const elements = {};
    const storage = new Map();
    const docListeners = {};
    const winListeners = {};
    const storageListeners = [];
    const runtimeListeners = [];
    const docElAttrs = new Map();
    let nowValue = overrides.now ?? 1_700_000_000_000;
    let currentHref = overrides.href || 'https://www.youtube.com/';

    function makeNode(tag, init = {}) {
      const attrs = new Map(Object.entries(init.attrs || {}));
      const node = {
        tagName: tag.toUpperCase(),
        id: init.id || '',
        textContent: init.textContent || '',
        href: init.href || '',
        style: init.style || {},
        children: [],
        attributes: attrs,
        getAttribute: (k) => (attrs.has(k) ? attrs.get(k) : null),
        setAttribute: (k, v) => attrs.set(k, String(v)),
        hasAttribute: (k) => attrs.has(k),
        removeAttribute: (k) => attrs.delete(k),
        appendChild: (c) => {
          node.children.push(c);
          return c;
        },
        addEventListener: () => {},
        querySelector: init.querySelector || (() => null),
        querySelectorAll: init.querySelectorAll || (() => []),
        remove: () => {
          if (node.id) delete elements[node.id];
        },
      };
      return node;
    }

    const register = (el) => {
      if (el && el.id) elements[el.id] = el;
      return el;
    };

    const documentElement = {
      appendChild: register,
      getAttribute: (k) => (docElAttrs.has(k) ? docElAttrs.get(k) : null),
      setAttribute: (k, v) => docElAttrs.set(k, String(v)),
      removeAttribute: (k) => docElAttrs.delete(k),
    };

    const queryAllImpl = (selector) => {
      if (typeof overrides.queryAll === 'function') {
        const custom = overrides.queryAll(selector);
        if (custom) return custom;
      }
      return [];
    };

    const document = {
      visibilityState: 'visible',
      getElementById: (id) => elements[id] || null,
      createElement: (tag) => makeNode(tag),
      querySelector: (selector) => {
        if (typeof overrides.query === 'function') {
          const hit = overrides.query(selector);
          if (hit !== undefined) return hit;
        }
        return queryAllImpl(selector)[0] || null;
      },
      querySelectorAll: queryAllImpl,
      head: { appendChild: register },
      body: { appendChild: register },
      documentElement,
      addEventListener: (name, fn) => {
        if (!docListeners[name]) docListeners[name] = [];
        docListeners[name].push(fn);
      },
    };

    const locationObj = {
      get href() {
        return currentHref;
      },
      set href(val) {
        currentHref = val;
      },
      get pathname() {
        return new URL(currentHref).pathname;
      },
      get origin() {
        return new URL(currentHref).origin;
      },
      replace: (url) => {
        replaced.push(url);
        currentHref = url;
      },
    };

    let storedControls = overrides.youtubeControls ?? null;

    const context = {
      console,
      URL,
      Date: { now: () => nowValue },
      setTimeout: (fn, delay) => {
        if (delay === 0 || delay === 500 || delay === 120) fn();
        return 0;
      },
      clearTimeout: () => {},
      setInterval: () => 0,
      fetch: overrides.fetch,
      MutationObserver: class {
        observe() {}
        disconnect() {}
      },
      location: locationObj,
      document,
      sessionStorage: {
        getItem: (key) => (storage.has(key) ? storage.get(key) : null),
        setItem: (key, value) => storage.set(key, String(value)),
        removeItem: (key) => storage.delete(key),
      },
      chrome: {
        runtime: {
          id: 'test-extension',
          lastError: null,
          sendMessage: (message, callback) => {
            messages.push(message);
            const response = overrides.onMessage
              ? overrides.onMessage(message)
              : null;
            callback(response);
          },
          onMessage: {
            addListener: (fn) => runtimeListeners.push(fn),
          },
        },
        storage: {
          local: {
            get: (_key, cb) => {
              if (typeof cb === 'function')
                cb({ youtubeControls: storedControls });
            },
          },
          onChanged: {
            addListener: (fn) => storageListeners.push(fn),
          },
        },
      },
    };

    const windowObj = {
      location: locationObj,
      addEventListener: (name, fn) => {
        if (!winListeners[name]) winListeners[name] = [];
        winListeners[name].push(fn);
      },
      postMessage: (msg, origin) => {
        postedMessages.push({ msg, origin });
      },
    };
    windowObj.window = windowObj;
    context.window = windowObj;
    context.globalThis = context;

    vm.createContext(context);
    vm.runInContext(controlsSource, context);
    vm.runInContext(source, context);

    return {
      api: context.__preelockYoutube,
      controlsApi: context.__preelockYoutubeControls,
      windowObj,
      messages,
      replaced,
      postedMessages,
      elements,
      docElAttrs,
      makeNode,
      setNow: (ms) => {
        nowValue = ms;
      },
      navigateTo: (url, eventName = 'yt-navigate-finish') => {
        currentHref = url;
        for (const fn of winListeners[eventName] || []) fn({});
      },
      fireWindow: (name, event = {}) => {
        for (const fn of winListeners[name] || []) fn(event);
      },
      fireDocument: (name, event = {}) => {
        for (const fn of docListeners[name] || []) fn(event);
      },
      fireStorageChange: (nextControls) => {
        storedControls = nextControls;
        for (const fn of storageListeners) {
          fn({ youtubeControls: { newValue: nextControls } }, 'local');
        }
      },
      firePolicyChanged: () => {
        for (const fn of runtimeListeners) {
          fn({ type: 'preelock_policy_changed' }, {}, () => {});
        }
      },
    };
  }

  it('injects and updates YouTube Controls CSS live via storage.onChanged and cleans up when turned off', async () => {
    const env = loadIntegrated({
      href: 'https://www.youtube.com/',
      youtubeControls: {
        home: { hideHomeFeed: true },
        player: { disableAnnotations: true },
        social: { hideComments: true },
        other: { hideTopHeader: true },
      },
      onMessage: (m) =>
        m.type === 'youtube_policy'
          ? { policy: null, browsingClosed: false }
          : {},
    });
    await settle();

    expect(env.docElAttrs.get('data-preelock-route')).toBe('home');
    const style = env.elements['preelock-yt-controls-style'];
    expect(style).toBeDefined();
    expect(style.textContent).toContain(
      'ytd-browse[page-subtype="home"] #contents',
    );
    expect(style.textContent).toContain('.ytp-cards-teaser');
    expect(style.textContent).toContain('ytd-comments');
    expect(style.textContent).toContain('--ytd-masthead-height:0px!important');

    // Turning every control off removes the style tag cleanly.
    env.fireStorageChange(env.controlsApi.normalizeYoutubeControls(null));
    await settle();
    expect(env.elements['preelock-yt-controls-style']).toBeUndefined();
  });

  it('redirects Homepage to Subscriptions across direct load, SPA, and popstate without loops, yielding to Watch Later and Study Mode', async () => {
    let policy = null;
    let browsingClosed = false;
    const env = loadIntegrated({
      href: 'https://www.youtube.com/',
      now: 1000,
      youtubeControls: { home: { redirectToSubscriptions: true } },
      onMessage: (m) => {
        if (m.type === 'youtube_policy') return { policy, browsingClosed };
        if (m.type === 'youtube_page_check') return { blocked: false };
        return { ok: true };
      },
    });
    await settle();
    expect(env.replaced).toEqual([
      'https://www.youtube.com/feed/subscriptions',
    ]);

    // Staying on Subscriptions or navigating to Watch does not redirect.
    env.navigateTo('https://www.youtube.com/watch?v=dQw4w9WgXcQ');
    await settle();
    expect(env.replaced).toHaveLength(1);

    // Immediate redirect inside 1500ms cooldown is suppressed to avoid loops.
    env.navigateTo('https://www.youtube.com/', 'yt-navigate-finish');
    await settle();
    expect(env.replaced).toHaveLength(1);

    // Browser Back/Forward (popstate) onto Homepage after cooldown redirects again.
    env.setNow(4000);
    env.navigateTo('https://www.youtube.com/', 'popstate');
    await settle();
    expect(env.replaced).toEqual([
      'https://www.youtube.com/feed/subscriptions',
      'https://www.youtube.com/feed/subscriptions',
    ]);

    // Watch Later precedence wins over Redirect to Subscriptions.
    env.setNow(7000);
    policy = {
      mode: 'allowlist',
      domains: ['github.com'],
      youtube_allow_watch_later: true,
    };
    browsingClosed = true;
    env.navigateTo('https://www.youtube.com/', 'yt-navigate-finish');
    await settle();
    expect(env.replaced[2]).toBe('https://www.youtube.com/playlist?list=WL');

    // Study mode does not redirect homepage to Subscriptions.
    env.setNow(10000);
    policy = {
      mode: 'blocklist',
      domains: [],
      youtube_study_mode: true,
      youtube_allowed_channels: ['@allowed'],
    };
    browsingClosed = false;
    expect(
      env.api.subscriptionsRedirectTarget(
        'https://www.youtube.com/',
        { home: { redirectToSubscriptions: true } },
        policy,
        browsingClosed,
      ),
    ).toBe(null);
  });

  it('enforces Hide Shorts directly on /shorts/* URLs and removes cover when disabled', async () => {
    const video = {
      tagName: 'VIDEO',
      muted: false,
      pause: () => {
        video.paused = true;
      },
      paused: false,
    };
    const env = loadIntegrated({
      href: 'https://www.youtube.com/shorts/shortVid123',
      youtubeControls: { recommendations: { hideShorts: true } },
      queryAll: (selector) => (selector === 'video' ? [video] : []),
      onMessage: (m) =>
        m.type === 'youtube_policy'
          ? { policy: null, browsingClosed: false }
          : {},
    });
    await settle();

    expect(env.elements['preelock-shorts-cover']).toBeDefined();
    expect(video.paused).toBe(true);
    expect(video.muted).toBe(true);

    // Turning Hide Shorts off removes the cover immediately.
    env.fireStorageChange({ recommendations: { hideShorts: false } });
    await settle();
    expect(env.elements['preelock-shorts-cover']).toBeUndefined();
  });

  it('disables Autoplay by clicking active autonav toggle and cancelling endscreen without pausing normal playback', async () => {
    const video = {
      tagName: 'VIDEO',
      muted: false,
      paused: false,
      pause: () => {
        video.paused = true;
      },
    };
    let autonavChecked = 'true';
    let autonavClicks = 0;
    let cancelClicks = 0;
    const autonavBtn = {
      getAttribute: (k) => (k === 'aria-checked' ? autonavChecked : null),
      click: () => {
        autonavClicks++;
        autonavChecked = 'false';
      },
    };
    const cancelBtn = {
      click: () => {
        cancelClicks++;
      },
    };

    const env = loadIntegrated({
      href: 'https://www.youtube.com/watch?v=allow12345',
      youtubeControls: { player: { disableAutoplay: true } },
      query: (selector) => {
        if (selector === '.ytp-autonav-toggle-button') return autonavBtn;
        if (selector === '.ytp-autonav-endscreen-upnext-cancel-button')
          return cancelBtn;
        if (selector === '.ytp-autonav-endscreen-upnext-container')
          return { style: { display: '' } };
        return undefined;
      },
      queryAll: (selector) => (selector === 'video' ? [video] : []),
      onMessage: (m) =>
        m.type === 'youtube_policy'
          ? { policy: null, browsingClosed: false }
          : {},
    });
    await settle();

    expect(autonavClicks).toBe(1);
    expect(autonavChecked).toBe('false');
    expect(cancelClicks).toBe(1);
    expect(video.paused).toBe(false);
    expect(
      env.postedMessages.some(
        (p) =>
          p.msg &&
          p.msg.source === 'preelock-content' &&
          p.msg.type === 'set-autoplay' &&
          p.msg.disabled === true,
      ),
    ).toBe(true);
  });

  it('applies dynamic DOM markers and clears blocked-channel card marks when a session ends', async () => {
    let policy = {
      id: 's1',
      mode: 'blocklist',
      domains: [],
      youtube_blocked_channels: ['@blocked'],
    };

    const makeMockCard = (childrenBySelector) => {
      const attrs = new Map();
      return {
        attributes: attrs,
        getAttribute: (k) => (attrs.has(k) ? attrs.get(k) : null),
        setAttribute: (k, v) => attrs.set(k, String(v)),
        hasAttribute: (k) => attrs.has(k),
        removeAttribute: (k) => attrs.delete(k),
        querySelector: (sel) => (childrenBySelector.one || {})[sel] || null,
        querySelectorAll: (sel) => (childrenBySelector.all || {})[sel] || [],
      };
    };

    const shortsChip = makeMockCard({});
    shortsChip.textContent = ' Shorts ';
    const exploreSection = makeMockCard({
      one: { '#guide-section-title': { textContent: 'Explore' } },
    });
    const mixCard = makeMockCard({
      one: { 'badge-shape, .badge-style-type-simple': { textContent: 'Mix' } },
    });
    const blockedCard = makeMockCard({
      all: {
        'a[href^="/@"], a[href^="/channel/"], a[href^="/c/"], a[href^="/user/"]':
          [{ getAttribute: () => '/@blocked' }],
      },
    });
    const allowedCard = makeMockCard({
      all: {
        'a[href^="/@"], a[href^="/channel/"], a[href^="/c/"], a[href^="/user/"]':
          [{ getAttribute: () => '/@allowed' }],
      },
    });

    const markedNodes = new Set([
      shortsChip,
      exploreSection,
      mixCard,
      blockedCard,
    ]);

    const env = loadIntegrated({
      href: 'https://www.youtube.com/results?search_query=music',
      youtubeControls: {
        navigation: { hideExploreAndTrending: true },
        recommendations: { hideShorts: true, hideMixes: true },
      },
      queryAll: (selector) => {
        if (selector === 'yt-chip-cloud-chip-renderer, chip-shape')
          return [shortsChip];
        if (selector === 'ytd-guide-section-renderer') return [exploreSection];
        if (
          selector ===
          'yt-lockup-view-model, ytd-rich-item-renderer, ytd-compact-video-renderer'
        )
          return [mixCard];
        if (selector.includes('ytd-channel-renderer'))
          return [blockedCard, allowedCard];
        if (selector === '[data-preelock-blocked-card]')
          return [...markedNodes].filter((n) =>
            n.hasAttribute('data-preelock-blocked-card'),
          );
        return [];
      },
      onMessage: (m) => {
        if (m.type === 'youtube_policy')
          return { policy, browsingClosed: false };
        if (m.type === 'youtube_page_check') return { blocked: false };
        return { ok: true };
      },
    });
    await settle();

    expect(shortsChip.getAttribute('data-preelock-hidden')).toBe('shorts');
    expect(exploreSection.getAttribute('data-preelock-hidden')).toBe('explore');
    expect(mixCard.getAttribute('data-preelock-hidden')).toBe('mix');
    expect(blockedCard.getAttribute('data-preelock-blocked-card')).toBe('true');
    expect(allowedCard.hasAttribute('data-preelock-blocked-card')).toBe(false);

    // Ending the session removes session blocked-card marks while user controls remain.
    policy = null;
    env.firePolicyChanged();
    await settle();
    expect(blockedCard.hasAttribute('data-preelock-blocked-card')).toBe(false);
    expect(shortsChip.getAttribute('data-preelock-hidden')).toBe('shorts');
  });

  it('marks signed-in subscribed channels sidebar section when Hide Subscriptions is enabled and spares Home/You/Watch Later', async () => {
    const subsSection = {
      attrs: new Map(),
      getAttribute(k) {
        return this.attrs.has(k) ? this.attrs.get(k) : null;
      },
      setAttribute(k, v) {
        this.attrs.set(k, String(v));
      },
      removeAttribute(k) {
        this.attrs.delete(k);
      },
      querySelector(sel) {
        if (sel.includes('a[href="/"]')) return null;
        if (sel.includes('/feed/trending')) return null;
        if (sel.includes('/@')) return { href: '/@veritasium' };
        return null;
      },
    };
    const primarySection = {
      attrs: new Map(),
      getAttribute(k) {
        return this.attrs.has(k) ? this.attrs.get(k) : null;
      },
      setAttribute(k, v) {
        this.attrs.set(k, String(v));
      },
      removeAttribute(k) {
        this.attrs.delete(k);
      },
      querySelector(sel) {
        if (sel.includes('a[href="/"]')) return { href: '/' };
        return null;
      },
    };
    const wlSection = {
      attrs: new Map(),
      getAttribute(k) {
        return this.attrs.has(k) ? this.attrs.get(k) : null;
      },
      setAttribute(k, v) {
        this.attrs.set(k, String(v));
      },
      removeAttribute(k) {
        this.attrs.delete(k);
      },
      querySelector(sel) {
        if (sel.includes('/playlist?list=WL'))
          return { href: '/playlist?list=WL' };
        return null;
      },
    };

    const exploreSection = {
      attrs: new Map(),
      getAttribute(k) {
        return this.attrs.has(k) ? this.attrs.get(k) : null;
      },
      setAttribute(k, v) {
        this.attrs.set(k, String(v));
      },
      removeAttribute(k) {
        this.attrs.delete(k);
      },
      querySelector(sel) {
        if (sel === '#guide-section-title') return { textContent: 'Explore' };
        if (sel.includes('/channel/UC-9-kyTW8ZkZNDHQJ6FgpwQ')) {
          return { href: '/channel/UC-9-kyTW8ZkZNDHQJ6FgpwQ' };
        }
        return null;
      },
    };

    const env = loadIntegrated({
      href: 'https://www.youtube.com/feed/subscriptions',
      youtubeControls: {
        navigation: { hideSubscriptions: true },
      },
      queryAll: (selector) => {
        if (selector === 'ytd-guide-section-renderer') {
          return [primarySection, subsSection, wlSection, exploreSection];
        }
        if (selector === '[data-preelock-hidden="subscriptions"]') {
          return [
            primarySection,
            subsSection,
            wlSection,
            exploreSection,
          ].filter(
            (n) => n.getAttribute('data-preelock-hidden') === 'subscriptions',
          );
        }
        return [];
      },
      onMessage: (m) =>
        m.type === 'youtube_policy'
          ? { policy: null, browsingClosed: false }
          : {},
    });
    await settle();

    expect(subsSection.getAttribute('data-preelock-hidden')).toBe(
      'subscriptions',
    );
    expect(primarySection.getAttribute('data-preelock-hidden')).toBe(null);
    expect(wlSection.getAttribute('data-preelock-hidden')).toBe(null);
    expect(exploreSection.getAttribute('data-preelock-hidden')).toBe(null);

    env.fireStorageChange({ navigation: { hideSubscriptions: false } });
    await settle();
    expect(subsSection.getAttribute('data-preelock-hidden')).toBe(null);
  });

  it('normalizes legacy both-on Subscriptions flags in the content script: Redirect wins on Home and Hide never applies', async () => {
    const makeSection = (querySelector) => ({
      attrs: new Map(),
      getAttribute(k) {
        return this.attrs.has(k) ? this.attrs.get(k) : null;
      },
      setAttribute(k, v) {
        this.attrs.set(k, String(v));
      },
      removeAttribute(k) {
        this.attrs.delete(k);
      },
      querySelector,
    });
    const subsSection = makeSection((sel) => {
      if (sel.includes('a[href="/"')) return null;
      if (sel.includes('/feed/trending')) return null;
      if (sel.includes('/@')) return { href: '/@veritasium' };
      return null;
    });
    const primarySection = makeSection((sel) => {
      if (sel.includes('a[href="/"')) return { href: '/' };
      return null;
    });
    const wlSection = makeSection((sel) => {
      if (sel.includes('/playlist?list=WL'))
        return { href: '/playlist?list=WL' };
      return null;
    });
    const exploreSection = makeSection((sel) => {
      if (sel === '#guide-section-title') return { textContent: 'Explore' };
      if (sel.includes('/channel/UC-9-kyTW8ZkZNDHQJ6FgpwQ')) {
        return { href: '/channel/UC-9-kyTW8ZkZNDHQJ6FgpwQ' };
      }
      return null;
    });

    const env = loadIntegrated({
      href: 'https://www.youtube.com/',
      youtubeControls: {
        home: { redirectToSubscriptions: true },
        navigation: { hideSubscriptions: true },
      },
      queryAll: (selector) => {
        if (selector === 'ytd-guide-section-renderer') {
          return [primarySection, subsSection, wlSection, exploreSection];
        }
        if (selector === '[data-preelock-hidden="subscriptions"]') {
          return [
            primarySection,
            subsSection,
            wlSection,
            exploreSection,
          ].filter(
            (n) => n.getAttribute('data-preelock-hidden') === 'subscriptions',
          );
        }
        return [];
      },
      onMessage: (m) => {
        if (m.type === 'youtube_policy')
          return { policy: null, browsingClosed: false };
        if (m.type === 'youtube_page_check') return { blocked: false };
        return {};
      },
    });
    await settle();

    // Redirect won on Home: the page moved to Subscriptions.
    expect(env.replaced).toEqual([
      'https://www.youtube.com/feed/subscriptions',
    ]);

    // Re-apply the same legacy both-on state on the Subscriptions
    // page: Hide must not apply after normalization.
    env.fireStorageChange({
      home: { redirectToSubscriptions: true },
      navigation: { hideSubscriptions: true },
    });
    await settle();
    expect(subsSection.getAttribute('data-preelock-hidden')).toBe(null);
    expect(primarySection.getAttribute('data-preelock-hidden')).toBe(null);
  });

  it('ignores stale page-bridge messages, stops on yt-navigate-start, resolves via oEmbed, and fails closed on channel timeout', async () => {
    const video = {
      tagName: 'VIDEO',
      muted: false,
      paused: false,
      pause: () => {
        video.paused = true;
      },
    };
    const policy = {
      id: 's-chan',
      mode: 'blocklist',
      domains: [],
      youtube_blocked_channels: ['@blocked'],
    };

    const env = loadIntegrated({
      href: 'https://www.youtube.com/watch?v=newVid999',
      now: 1000,
      queryAll: (selector) => (selector === 'video' ? [video] : []),
      fetch: async () => ({
        ok: true,
        json: async () => ({ author_url: 'https://www.youtube.com/@other' }),
      }),
      onMessage: (m) => {
        if (m.type === 'youtube_policy')
          return { policy, browsingClosed: false };
        if (m.type === 'youtube_page_check') return { blocked: false };
        if (m.type === 'youtube_check') {
          return youtubeWatchDecision(
            m.url,
            m.channel ?? null,
            policy,
            Boolean(m.unidentified),
            m.watchLaterMember ?? null,
          );
        }
        return { ok: true };
      },
    });
    await settle();

    // First video resolved via oEmbed as @other (allowed) and guard cleared.
    const oembedCheck = env.messages.find(
      (m) => m.type === 'youtube_check' && m.videoId === 'newVid999',
    );
    expect(oembedCheck.channels).toContain('@other');

    // Navigate via SPA to another video where DOM & oEmbed give no channel.
    env.setNow(2000);
    // Simulate a blocked channel message for the OLD videoId arriving after navigation -> MUST be ignored!
    env.navigateTo(
      'https://www.youtube.com/watch?v=nextVid12',
      'yt-navigate-start',
    );
    expect(video.paused).toBe(true);
    env.fireWindow('message', {
      source: undefined, // mismatched source rejected cleanly
    });

    // Inject valid postMessage for the current videoId with @blocked -> must block.
    // Note: in our test harness windowObj === context.window; attach event.source & origin.
    const nextEnv = loadIntegrated({
      href: 'https://www.youtube.com/watch?v=nextVid12',
      now: 2000,
      queryAll: (selector) => (selector === 'video' ? [video] : []),
      fetch: async () => ({ ok: false }),
      onMessage: (m) => {
        if (m.type === 'youtube_policy')
          return { policy, browsingClosed: false };
        if (m.type === 'youtube_page_check') return { blocked: false };
        if (m.type === 'youtube_check') {
          return youtubeWatchDecision(
            m.url,
            m.channel ?? null,
            policy,
            Boolean(m.unidentified),
            m.watchLaterMember ?? null,
          );
        }
        return { ok: true };
      },
    });
    await settle();

    // Still unidentified inside 5000ms -> pending, guard active, not blocked yet.
    expect(nextEnv.elements['preelock-yt-guard']).toBeDefined();
    expect(nextEnv.messages.some((m) => m.type === 'youtube_block')).toBe(
      false,
    );

    // Past DETECTION_TIMEOUT_MS (5000ms) -> fails closed and blocks!
    nextEnv.setNow(8000);
    nextEnv.fireWindow('yt-page-data-updated');
    await settle();
    const timedOutCheck = nextEnv.messages
      .filter((m) => m.type === 'youtube_check')
      .pop();
    expect(timedOutCheck.unidentified).toBe(true);
    expect(nextEnv.messages.some((m) => m.type === 'youtube_block')).toBe(true);
  });

  it('accepts matching page-bridge identity messages and ignores stale videoId bridge messages', async () => {
    const policy = {
      id: 's-bridge',
      mode: 'blocklist',
      domains: [],
      youtube_blocked_channels: ['@blocked'],
    };
    const env = loadIntegrated({
      href: 'https://www.youtube.com/watch?v=currVid01',
      now: 2000,
      fetch: async () => ({ ok: false }),
      onMessage: (m) => {
        if (m.type === 'youtube_policy')
          return { policy, browsingClosed: false };
        if (m.type === 'youtube_page_check') return { blocked: false };
        if (m.type === 'youtube_check') {
          return youtubeVideoDecision(
            m.url,
            m.channels ?? (m.channel ? [m.channel] : []),
            policy,
            Boolean(m.unidentified),
            m.watchLaterMember ?? null,
          );
        }
        return { ok: true };
      },
    });
    await settle();

    // Mismatched videoId from a previous video must be ignored cleanly.
    env.fireWindow('message', {
      source: env.windowObj,
      origin: 'https://www.youtube.com',
      data: {
        source: 'preelock-page',
        type: 'video',
        videoId: 'oldVid999',
        channelId: 'UCBlocked12345678901234',
        ownerUrl: '/@blocked',
      },
    });
    await settle();
    expect(env.messages.some((m) => m.type === 'youtube_block')).toBe(false);

    // Matching videoId triggers immediate verification and blocks @blocked.
    env.fireWindow('message', {
      source: env.windowObj,
      origin: 'https://www.youtube.com',
      data: {
        source: 'preelock-page',
        type: 'video',
        videoId: 'currVid01',
        channelId: 'UCBlocked12345678901234',
        ownerUrl: '/@blocked',
      },
    });
    await settle();
    expect(env.messages.some((m) => m.type === 'youtube_block')).toBe(true);
  });

  it('ignores unscoped global #owner/recommendation DOM and stale ytd-watch-flexy during SPA transitions', async () => {
    const policy = {
      id: 's-scope',
      mode: 'blocklist',
      domains: [],
      youtube_blocked_channels: ['@blocked'],
    };
    const video = {
      tagName: 'VIDEO',
      muted: false,
      paused: false,
      pause: () => {
        video.paused = true;
      },
      play: () => {
        video.paused = false;
        return Promise.resolve();
      },
    };
    const staleFlexy = {
      hasAttribute: (name) => name === 'hidden',
      getAttribute: (name) => (name === 'video-id' ? 'oldVid01' : null),
    };
    const env = loadIntegrated({
      href: 'https://www.youtube.com/watch?v=target99',
      now: 1000,
      query: (selector) => {
        if (selector.includes('ytd-watch-flexy')) return staleFlexy;
        if (selector.includes('meta[') || selector.includes('link['))
          return null;
        return undefined;
      },
      queryAll: (selector) => {
        if (selector === 'video') return [video];
        if (
          selector.includes('#owner') ||
          selector.includes('ytd-channel-name') ||
          selector.includes('ytd-video-owner-renderer')
        ) {
          return [{ getAttribute: () => '/@innocent' }];
        }
        return [];
      },
      fetch: async () => ({
        ok: true,
        json: async () => ({ author_url: 'https://www.youtube.com/@blocked' }),
      }),
      onMessage: (m) => {
        if (m.type === 'youtube_policy')
          return { policy, browsingClosed: false };
        if (m.type === 'youtube_page_check') return { blocked: false };
        if (m.type === 'youtube_check') {
          return youtubeVideoDecision(
            m.url,
            m.channels ?? [],
            policy,
            Boolean(m.unidentified),
            m.watchLaterMember ?? null,
          );
        }
        return { ok: true };
      },
    });
    await settle();

    // oEmbed MUST still resolve @blocked instead of getting skipped by stale global DOM.
    const check = env.messages.find(
      (m) => m.type === 'youtube_check' && m.videoId === 'target99',
    );
    expect(check.channels).toContain('@blocked');
    expect(env.messages.some((m) => m.type === 'youtube_block')).toBe(true);
    expect(video.paused).toBe(true);
  });

  it('re-evaluates when page-bridge supplies new identity after initial allow and supports internal player swap without URL change', async () => {
    const policy = {
      id: 's-swap',
      mode: 'blocklist',
      domains: [],
      youtube_blocked_channels: ['UCBlocked12345678901234'],
    };
    const mutedAlready = {
      tagName: 'VIDEO',
      muted: true,
      paused: false,
      pause: () => {
        mutedAlready.paused = true;
      },
      play: () => {
        mutedAlready.paused = false;
        return Promise.resolve();
      },
    };
    const env = loadIntegrated({
      href: 'https://www.youtube.com/@veritasium/live',
      now: 1000,
      queryAll: (selector) => (selector === 'video' ? [mutedAlready] : []),
      fetch: async () => ({ ok: false }),
      onMessage: (m) => {
        if (m.type === 'youtube_policy')
          return { policy, browsingClosed: false };
        if (m.type === 'youtube_page_check') return { blocked: false };
        if (m.type === 'youtube_check') {
          return youtubeVideoDecision(
            m.url,
            m.channels ?? [],
            policy,
            Boolean(m.unidentified),
            m.watchLaterMember ?? null,
          );
        }
        return { ok: true };
      },
    });
    await settle();

    // Later bridge message arrives with UCBlocked -> MUST NOT drop even if allowedKey was set.
    env.fireWindow('message', {
      source: env.windowObj,
      origin: 'https://www.youtube.com',
      data: {
        source: 'preelock-page',
        type: 'video',
        videoId: 'liveVid01',
        channelId: 'UCBlocked12345678901234',
        ownerUrl: '/@veritasium',
      },
    });
    await settle();
    expect(env.messages.some((m) => m.type === 'youtube_block')).toBe(true);

    // Separate test for internal player swap on /watch without URL change + per-video mute preservation.
    const unmutedVid = {
      tagName: 'VIDEO',
      muted: false,
      paused: false,
      pause: () => {
        unmutedVid.paused = true;
      },
      play: () => {
        unmutedVid.paused = false;
        return Promise.resolve();
      },
    };
    const env2 = loadIntegrated({
      href: 'https://www.youtube.com/watch?v=allowedA1',
      now: 1000,
      queryAll: (selector) =>
        selector === 'video' ? [unmutedVid, mutedAlready] : [],
      fetch: async () => ({ ok: false }),
      onMessage: (m) => {
        if (m.type === 'youtube_policy')
          return { policy, browsingClosed: false };
        if (m.type === 'youtube_page_check') return { blocked: false };
        if (m.type === 'youtube_check') {
          return youtubeVideoDecision(
            m.url,
            m.channels ?? [],
            policy,
            Boolean(m.unidentified),
            m.watchLaterMember ?? null,
          );
        }
        return { ok: true };
      },
    });
    // Identify allowedA1 as an allowed channel.
    env2.fireWindow('message', {
      source: env2.windowObj,
      origin: 'https://www.youtube.com',
      data: {
        source: 'preelock-page',
        type: 'video',
        videoId: 'allowedA1',
        channelId: 'UCAllowed12345678901234',
        ownerUrl: '/@allowed',
      },
    });
    await settle();
    expect(env2.elements['preelock-yt-guard']).toBeUndefined();
    expect(unmutedVid.muted).toBe(false);
    expect(mutedAlready.muted).toBe(true);

    // Internal player swap (URL stays /watch?v=allowedA1) switches player to blockedB2.
    env2.fireWindow('message', {
      source: env2.windowObj,
      origin: 'https://www.youtube.com',
      data: {
        source: 'preelock-page',
        type: 'video',
        videoId: 'blockedB2',
        channelId: 'UCBlocked12345678901234',
        ownerUrl: '/@veritasium',
        urlVideoId: 'allowedA1',
        playerSwapped: true,
      },
    });
    expect(env2.elements['preelock-yt-guard']).toBeDefined();
    expect(unmutedVid.paused).toBe(true);
    await settle();
    expect(env2.messages.some((m) => m.type === 'youtube_block')).toBe(true);
  });

  it('catches synchronous play/playing/loadedmetadata on route change, blocks inline previews, and clears guard on session end', async () => {
    let activePolicy = {
      id: 's-sess',
      mode: 'blocklist',
      domains: [],
      youtube_blocked_channels: ['@blocked'],
    };
    const mainVideo = {
      tagName: 'VIDEO',
      muted: false,
      paused: false,
      pause: () => {
        mainVideo.paused = true;
      },
      play: () => {
        mainVideo.paused = false;
        return Promise.resolve();
      },
    };
    const previewVideo = {
      tagName: 'VIDEO',
      muted: false,
      paused: false,
      closest: (sel) => (sel === '#inline-preview-player' ? {} : null),
      pause: () => {
        previewVideo.paused = true;
      },
    };
    const env = loadIntegrated({
      href: 'https://www.youtube.com/watch?v=allow0001',
      now: 1000,
      query: (sel) =>
        sel === '#inline-preview-player video' ? previewVideo : undefined,
      queryAll: (sel) => (sel === 'video' ? [mainVideo] : []),
      fetch: async () => ({ ok: false }),
      onMessage: (m) => {
        if (m.type === 'youtube_policy')
          return { policy: activePolicy, browsingClosed: false };
        if (m.type === 'youtube_page_check') return { blocked: false };
        if (m.type === 'youtube_check') {
          return youtubeVideoDecision(
            m.url,
            m.channels ?? [],
            activePolicy,
            Boolean(m.unidentified),
            m.watchLaterMember ?? null,
          );
        }
        return { ok: true };
      },
    });
    env.fireWindow('message', {
      source: env.windowObj,
      origin: 'https://www.youtube.com',
      data: {
        source: 'preelock-page',
        type: 'video',
        videoId: 'allow0001',
        channelId: 'UCAllowed12345678901234',
        ownerUrl: '/@allowed',
      },
    });
    await settle();
    expect(env.elements['preelock-yt-guard']).toBeUndefined();

    // Preview video belonging to blocked channel must pause preview without affecting main player.
    env.fireWindow('message', {
      source: env.windowObj,
      origin: 'https://www.youtube.com',
      data: {
        source: 'preelock-page',
        type: 'preview-video',
        videoId: 'prev99',
        channelId: 'UCBlocked12345678901234',
        ownerUrl: '/@blocked',
      },
    });
    await settle();
    expect(previewVideo.paused).toBe(true);
    expect(mainVideo.paused).toBe(false);

    // Simulate pushState route change + immediate <video> 'playing' before async IPC returns.
    env.windowObj.location.href = 'https://www.youtube.com/watch?v=nextVid02';
    env.fireDocument('playing', { target: mainVideo });
    expect(mainVideo.paused).toBe(true);
    expect(mainVideo.muted).toBe(true);
    expect(env.elements['preelock-yt-guard']).toBeDefined();

    // Session ends -> guard vanishes cleanly and allows playback again.
    activePolicy = null;
    env.fireWindow('yt-page-data-updated');
    await settle();
    expect(env.elements['preelock-yt-guard']).toBeUndefined();
    expect(mainVideo.muted).toBe(false);
  });

  it('page-bridge emits nav on history.pushState, suppresses stale pre-settle player videoId, and detects internal player swap', () => {
    const bridgeSource = readFileSync(
      path.join(here, 'page-bridge.js'),
      'utf8',
    );
    const posted = [];
    const intervals = [];
    let currentHref = 'https://www.youtube.com/watch?v=vidA1';
    let currentResp = {
      videoDetails: {
        videoId: 'vidA1',
        channelId: 'UCAllowed12345678901234',
        author: 'Allowed',
      },
      microformat: {
        playerMicroformatRenderer: { canonicalBaseUrl: '/@allowed' },
      },
    };
    let currentVData = { video_id: 'vidA1', author: 'Allowed' };
    const player = {
      getPlayerResponse: () => currentResp,
      getVideoData: () => currentVData,
    };
    const historyObj = {
      pushState(_state, _title, nextUrl) {
        currentHref = new URL(nextUrl, currentHref).href;
      },
      replaceState(_state, _title, nextUrl) {
        currentHref = new URL(nextUrl, currentHref).href;
      },
    };
    const winObj = {
      location: {
        get href() {
          return currentHref;
        },
        get origin() {
          return 'https://www.youtube.com';
        },
      },
      history: historyObj,
      postMessage: (data) => posted.push(data),
      addEventListener: () => {},
    };
    const ctx = {
      URL,
      window: winObj,
      history: historyObj,
      location: winObj.location,
      document: {
        querySelector: (sel) => (sel === '#movie_player' ? player : null),
        addEventListener: () => {},
      },
      setInterval: (fn) => {
        intervals.push(fn);
        return intervals.length;
      },
      setTimeout: () => 0,
    };
    vm.createContext(ctx);
    vm.runInContext(bridgeSource, ctx);

    const firstVideo = posted.filter((m) => m.type === 'video').pop();
    expect(firstVideo.videoId).toBe('vidA1');
    expect(firstVideo.channelId).toBe('UCAllowed12345678901234');
    expect(firstVideo.playerSwapped).toBe(false);

    // Internal player swap WITHOUT URL change -> playerSwapped: true emitted!
    posted.length = 0;
    currentResp = {
      videoDetails: {
        videoId: 'vidB2',
        channelId: 'UCBlocked12345678901234',
        author: 'Blocked',
      },
      microformat: {
        playerMicroformatRenderer: { canonicalBaseUrl: '/@blocked' },
      },
    };
    currentVData = { video_id: 'vidB2', author: 'Blocked' };
    for (const fn of intervals) fn();
    const swapMsg = posted.find((m) => m.type === 'video');
    expect(swapMsg).toBeDefined();
    expect(swapMsg.videoId).toBe('vidB2');
    expect(swapMsg.channelId).toBe('UCBlocked12345678901234');
    expect(swapMsg.playerSwapped).toBe(true);

    // pushState to vidC3 while player STILL temporarily holds vidB2 -> must NOT emit vidB2 for vidC3!
    posted.length = 0;
    historyObj.pushState({}, '', '/watch?v=vidC3');
    const navMsg = posted.find((m) => m.type === 'nav');
    expect(navMsg.href).toBe('https://www.youtube.com/watch?v=vidC3');
    expect(
      posted.some((m) => m.type === 'video' && m.videoId === 'vidB2'),
    ).toBe(false);
  });

  it('activates the playback guard on initial cold load before youtube_page_check finishes', async () => {
    const activePolicy = {
      id: 's-cold',
      mode: 'blocklist',
      domains: [],
      youtube_blocked_channels: ['@blocked'],
    };
    const coldVideo = {
      tagName: 'VIDEO',
      muted: false,
      paused: false,
      pause: () => {
        coldVideo.paused = true;
      },
      play: () => {
        coldVideo.paused = false;
        return Promise.resolve();
      },
    };
    let guardedBeforePageCheck = false;
    let env = null;
    env = loadIntegrated({
      href: 'https://www.youtube.com/watch?v=cold9991',
      queryAll: (sel) => (sel === 'video' ? [coldVideo] : []),
      fetch: async () => ({ ok: false }),
      onMessage: (m) => {
        if (m.type === 'youtube_policy') {
          return { policy: activePolicy, browsingClosed: false };
        }
        if (m.type === 'youtube_page_check') {
          guardedBeforePageCheck =
            Boolean(env?.elements?.['preelock-yt-guard']) && coldVideo.paused;
          return { blocked: false };
        }
        if (m.type === 'youtube_check') {
          return youtubeVideoDecision(
            m.url,
            m.channels ?? [],
            activePolicy,
            Boolean(m.unidentified),
            m.watchLaterMember ?? null,
          );
        }
        return { ok: true };
      },
    });
    await settle();
    expect(guardedBeforePageCheck).toBe(true);

    // Allowed channel response lifts the cold-load guard cleanly.
    env.fireWindow('message', {
      source: env.windowObj,
      origin: 'https://www.youtube.com',
      data: {
        source: 'preelock-page',
        type: 'video',
        videoId: 'cold9991',
        channelId: 'UCAllowed12345678901234',
        ownerUrl: '/@allowed',
      },
    });
    await settle();
    expect(env.elements['preelock-yt-guard']).toBeUndefined();
  });
});
