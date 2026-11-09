import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
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
    location: { href: overrides.href || 'https://www.youtube.com/watch?v=x' },
    document: {
      querySelector: () => overrides.player ?? null,
      querySelectorAll: () => overrides.ownerNodes ?? [],
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
  return { api: context.__preelockYoutube, messages, context };
}
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
    await Promise.resolve();
    await new Promise((resolve) => setTimeout(resolve, 0));
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
    await Promise.resolve();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(messages.some((m) => m.type === 'youtube_block')).toBe(false);
  });
});
