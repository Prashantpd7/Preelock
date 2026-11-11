import { describe, it, expect } from 'vitest';
import {
  blockDetail,
  channelBlockReason,
  channelKeyFromPath,
  compileRules,
  extractChannelKey,
  isChannelLivePath,
  isWatchLaterUrl,
  leaseValid,
  normalizeYouTubeChannel,
  ruleMatches,
  searchPageQuery,
  shouldRedirectToWatchLater,
  urlBlockedByPolicy,
  watchLaterOnly,
  watchLaterRedirectUrl,
  youtubeVideoDecision,
  youtubeWatchBlocked,
  youtubeWatchDecision,
} from './rules.js';
describe('real browser policy rule generation', () => {
  it('makes blocklist domain rules with subdomain matching', () => {
    const rules = compileRules({
      mode: 'blocklist',
      domains: ['example.com', 'reddit.com'],
    });
    expect(rules).toHaveLength(2);
    expect(rules[0].condition.requestDomains).toEqual(['example.com']);
    expect(rules[0].action.type).toBe('block');
  });
  it('allows only selected domains and recovery loopback in strict mode', () => {
    const rules = compileRules({ mode: 'allowlist', domains: ['docs.rs'] });
    // Allow rules (priority 2) for the domain + localhost, then a
    // catch-all block rule (priority 1).
    const allows = rules.filter((r) => r.action.type === 'allow');
    const blocks = rules.filter((r) => r.action.type === 'block');
    expect(
      allows.some((r) => r.condition.requestDomains.includes('docs.rs')),
    ).toBe(true);
    expect(
      allows.some((r) => r.condition.requestDomains.includes('127.0.0.1')),
    ).toBe(true);
    expect(blocks).toHaveLength(1);
    expect(blocks[0].condition.regexFilter).toBe('^(https?|wss?)://');
    expect(blocks[0].priority).toBe(1);
    expect(allows[0].priority).toBe(2);
  });
  it('allowlist blocks navigation only so allowed pages load their assets', () => {
    const rules = compileRules({ mode: 'allowlist', domains: ['docs.rs'] });
    for (const rule of rules) {
      expect(rule.condition.resourceTypes).toEqual(['main_frame', 'sub_frame']);
    }
  });
  it('offline keeps filtering every request type', () => {
    const rules = compileRules({ mode: 'offline', domains: [] });
    const blocks = rules.filter((r) => r.action.type === 'block');
    expect(blocks).toHaveLength(1);
    expect(blocks[0].condition.resourceTypes).toContain('main_frame');
    expect(blocks[0].condition.resourceTypes).toContain('stylesheet');
    for (const rule of rules.filter((r) => r.action.type === 'allow')) {
      expect(rule.condition.resourceTypes).toContain('xmlhttprequest');
    }
    const [blockRule] = compileRules({
      mode: 'blocklist',
      domains: ['example.com'],
    });
    expect(blockRule.condition.resourceTypes).toContain('main_frame');
    expect(blockRule.condition.resourceTypes).toContain('stylesheet');
  });
  it('offline mode does not retain approved public destinations', () => {
    const rules = compileRules({ mode: 'offline', domains: ['docs.rs'] });
    const allows = rules.filter((r) => r.action.type === 'allow');
    expect(
      allows.some((r) => r.condition.requestDomains.includes('docs.rs')),
    ).toBe(false);
    expect(
      allows.some((r) => r.condition.requestDomains.includes('127.0.0.1')),
    ).toBe(true);
  });
  it('removes every rule for cleanup and rejects malformed domains', () => {
    expect(compileRules(null)).toEqual([]);
    expect(() =>
      compileRules({ mode: 'blocklist', domains: ['*.com'] }),
    ).toThrow();
    expect(() =>
      compileRules({ mode: 'blocklist', domains: ['not a domain'] }),
    ).toThrow();
  });
  it('shares the Rust matcher: plain, wildcard and path rules', () => {
    expect(ruleMatches('example.com', 'example.com', '/')).toBe(true);
    expect(ruleMatches('example.com', 'www.example.com', '/x')).toBe(true);
    expect(ruleMatches('example.com', 'notexample.com', '/')).toBe(false);
    expect(ruleMatches('*.example.com', 'www.example.com', '/')).toBe(true);
    expect(ruleMatches('*.example.com', 'example.com', '/')).toBe(false);
    expect(ruleMatches('youtube.com/shorts', 'youtube.com', '/shorts')).toBe(
      true,
    );
    expect(
      ruleMatches('youtube.com/shorts', 'youtube.com', '/shorts/abc'),
    ).toBe(true);
    expect(ruleMatches('youtube.com/shorts', 'youtube.com', '/shortsabc')).toBe(
      false,
    );
    expect(ruleMatches('youtube.com/shorts', 'youtube.com', '/')).toBe(false);
    expect(ruleMatches('search:chess', 'google.com', '/search')).toBe(false);
  });
  it('compiles wildcard and path rules as regex rules, plain as domains', () => {
    const rules = compileRules({
      mode: 'blocklist',
      domains: ['example.com', '*.example.com', 'youtube.com/shorts'],
    });
    expect(rules).toHaveLength(3);
    expect(rules[0].condition.requestDomains).toEqual(['example.com']);
    expect(rules[1].condition.regexFilter).toContain('example');
    expect(rules[2].condition.regexFilter).toContain('shorts');
    const blocked = new RegExp(rules[2].condition.regexFilter);
    expect(blocked.test('https://www.youtube.com/shorts/abc')).toBe(true);
    expect(blocked.test('https://youtube.com/shortsabc')).toBe(false);
    expect(blocked.test('https://youtube.com/')).toBe(false);
  });
  it('blocks keyword search pages and adult overlay pages by URL', () => {
    const policy = {
      mode: 'blocklist',
      domains: ['example.com'],
      keywords: ['chess'],
      extra_blocked: ['pornhub.com'],
    };
    expect(
      urlBlockedByPolicy(
        'https://www.google.com/search?q=chess+openings',
        policy,
      ),
    ).toBe(true);
    expect(
      urlBlockedByPolicy('https://www.google.com/search?q=checkers', policy),
    ).toBe(false);
    expect(
      urlBlockedByPolicy(
        'https://www.youtube.com/results?search_query=chess',
        policy,
      ),
    ).toBe(true);
    expect(
      urlBlockedByPolicy('https://www.youtube.com/watch?v=abc', policy),
    ).toBe(false);
    expect(urlBlockedByPolicy('https://pornhub.com/', policy)).toBe(true);
    expect(searchPageQuery('https://www.bing.com/search?q=chess')?.query).toBe(
      'chess',
    );
    expect(searchPageQuery('https://duckduckgo.com/?q=chess')?.query).toBe(
      'chess',
    );
    expect(
      searchPageQuery('https://duckduckgo.com/?q=chess&ia=web')?.query,
    ).toBe('chess');
    expect(searchPageQuery('https://www.google.com/maps?q=chess')).toBe(null);
    const allow = {
      mode: 'allowlist',
      domains: ['pornhub.com', 'google.com'],
      keywords: ['chess'],
      extra_blocked: ['pornhub.com'],
    };
    expect(urlBlockedByPolicy('https://pornhub.com/', allow)).toBe(true);
    expect(
      urlBlockedByPolicy('https://www.google.com/search?q=chess', allow),
    ).toBe(true);
    expect(urlBlockedByPolicy('https://www.google.com/', allow)).toBe(false);
  });
  it('applies exception precedence: exception over block and keyword, never over adult or offline', () => {
    const policy = {
      mode: 'blocklist',
      domains: ['youtube.com', 'example.com'],
      keywords: ['chess'],
      extra_blocked: ['pornhub.com'],
      exceptions: ['youtube.com', 'example.com/docs'],
    };
    // Exception overrides a blocklist rule, including subdomains.
    expect(urlBlockedByPolicy('https://youtube.com/', policy)).toBe(false);
    expect(
      urlBlockedByPolicy('https://www.youtube.com/watch?v=x', policy),
    ).toBe(false);
    // A path exception allows only that path, not the whole domain.
    expect(urlBlockedByPolicy('https://example.com/docs/intro', policy)).toBe(
      false,
    );
    expect(urlBlockedByPolicy('https://example.com/other', policy)).toBe(true);
    // Exception on the search engine's host exempts its keyword blocks.
    const searchPolicy = {
      mode: 'blocklist',
      domains: [],
      keywords: ['chess'],
      exceptions: ['google.com'],
    };
    expect(
      urlBlockedByPolicy('https://www.google.com/search?q=chess', searchPolicy),
    ).toBe(false);
    expect(
      urlBlockedByPolicy('https://www.bing.com/search?q=chess', searchPolicy),
    ).toBe(true);
    // Exception does NOT override the adult overlay.
    const adultPolicy = {
      ...policy,
      exceptions: ['pornhub.com'],
    };
    expect(urlBlockedByPolicy('https://pornhub.com/', adultPolicy)).toBe(true);
    // Exception does NOT override offline mode.
    expect(
      urlBlockedByPolicy('https://youtube.com/', {
        ...policy,
        mode: 'offline',
      }),
    ).toBe(true);
    // Exception does NOT open anything in allowlist mode.
    const allow = {
      mode: 'allowlist',
      domains: ['docs.rs'],
      exceptions: ['github.com'],
    };
    expect(urlBlockedByPolicy('https://github.com/', allow)).toBe(true);
    expect(urlBlockedByPolicy('https://docs.rs/', allow)).toBe(false);
    // Local files are never blocked, with or without exceptions.
    expect(urlBlockedByPolicy('file:///Users/x/notes.html', policy)).toBe(
      false,
    );
  });
  it('compiles exception allows above block rules and below the adult overlay', () => {
    const rules = compileRules({
      mode: 'blocklist',
      domains: ['youtube.com'],
      extra_blocked: ['pornhub.com'],
      exceptions: ['youtube.com'],
    });
    const blocks = rules.filter((r) => r.action.type === 'block');
    const allows = rules.filter((r) => r.action.type === 'allow');
    expect(allows).toHaveLength(1);
    expect(allows[0].priority).toBe(2);
    expect(allows[0].condition.requestDomains).toEqual(['youtube.com']);
    expect(blocks.some((r) => r.priority === 1)).toBe(true);
    expect(blocks.some((r) => r.priority === 3)).toBe(true);
    // Adult domain that is also a block rule still gets a priority-3 block.
    const overlap = compileRules({
      mode: 'blocklist',
      domains: ['pornhub.com'],
      extra_blocked: ['pornhub.com'],
      exceptions: ['pornhub.com'],
    });
    expect(
      overlap.filter((r) => r.action.type === 'block' && r.priority === 3),
    ).toHaveLength(1);
    // Allowlist mode adds no allow for the excepted (non-listed) domain.
    expect(
      compileRules({
        mode: 'allowlist',
        domains: ['docs.rs'],
        exceptions: ['github.com'],
      }).filter(
        (r) =>
          r.action.type === 'allow' &&
          (r.condition.requestDomains || []).includes('github.com'),
      ),
    ).toHaveLength(0);
    expect(() =>
      compileRules({
        mode: 'blocklist',
        domains: [],
        exceptions: ['search:chess'],
      }),
    ).toThrow();
  });
  it('rejects expired and excessively long policy leases', () => {
    expect(leaseValid({ expires_at: 1000 }, 2000)).toBe(false);
    expect(leaseValid({ expires_at: 100000 }, 2000)).toBe(false);
    expect(leaseValid({ expires_at: 5000 }, 2000)).toBe(true);
  });
});
describe('YouTube channel rules', () => {
  it('normalizes every channel entry form, mirroring Rust', () => {
    expect(normalizeYouTubeChannel('@Some_Handle-1')).toBe('@some_handle-1');
    expect(
      normalizeYouTubeChannel('https://www.youtube.com/@SomeHandle/'),
    ).toBe('@somehandle');
    expect(normalizeYouTubeChannel('youtube.com/@handle/videos')).toBe(
      '@handle',
    );
    expect(
      normalizeYouTubeChannel('youtube.com/channel/UCabcdefghijklmnopqrstuv'),
    ).toBe('UCabcdefghijklmnopqrstuv');
    expect(normalizeYouTubeChannel('UCabcdefghijklmnopqrstuv')).toBe(
      'UCabcdefghijklmnopqrstuv',
    );
    expect(normalizeYouTubeChannel('https://youtube.com/c/SomeName')).toBe(
      'c:somename',
    );
    expect(normalizeYouTubeChannel('youtube.com/user/SomeName')).toBe(
      'user:somename',
    );
    expect(normalizeYouTubeChannel('c:somename')).toBe('c:somename');
    for (const bad of [
      '',
      'youtube.com',
      'youtube.com/watch?v=abc',
      'youtube.com/results?search_query=cats',
      'youtube.com/playlist?list=WL',
      'youtube.com/shorts/abc',
      'https://example.com/@handle',
      'https://youtu.be/abc',
      '@',
      '@ab',
      'UCshort',
      'not a channel',
      'handle',
    ]) {
      expect(normalizeYouTubeChannel(bad)).toBe(null);
    }
  });
  it('reads channel keys from paths and owner-chip hrefs', () => {
    expect(channelKeyFromPath('/@Handle/videos')).toBe('@handle');
    expect(channelKeyFromPath('/channel/UCabcdefghijklmnopqrstuv')).toBe(
      'UCabcdefghijklmnopqrstuv',
    );
    expect(channelKeyFromPath('/c/SomeName')).toBe('c:somename');
    expect(channelKeyFromPath('/user/SomeName')).toBe('user:somename');
    expect(channelKeyFromPath('/watch')).toBe(null);
    expect(channelKeyFromPath('/')).toBe(null);
    expect(
      extractChannelKey([
        '/watch?v=x',
        'https://www.youtube.com/@Owner',
        '/channel/UCabcdefghijklmnopqrstuv',
      ]),
    ).toBe('@owner');
    expect(extractChannelKey(['/watch?v=x', 'https://example.com/@x'])).toBe(
      null,
    );
    expect(extractChannelKey([])).toBe(null);
  });
  it('recognizes only the exact Watch Later playlist route', () => {
    // The one allowed route, on every main YouTube host spelling.
    for (const url of [
      'https://www.youtube.com/playlist?list=WL',
      'https://youtube.com/playlist?list=WL',
      'https://m.youtube.com/playlist?list=WL',
      'https://WWW.YOUTUBE.COM/playlist?list=WL',
      'http://www.youtube.com/playlist?list=WL',
      'https://www.youtube.com/playlist/?list=WL',
      'https://www.youtube.com/playlist?index=2&list=WL',
      'https://www.youtube.com/playlist?list=WL&index=3',
    ])
      expect(isWatchLaterUrl(url)).toBe(true);
    // A video URL is never the playlist, even carrying list=WL.
    expect(isWatchLaterUrl('https://www.youtube.com/watch?v=x&list=WL')).toBe(
      false,
    );
    // Exact parameter, exact identifier: lookalikes are not Watch Later.
    for (const url of [
      'https://www.youtube.com/playlist?list=WLxyz',
      'https://www.youtube.com/playlist?list=wl',
      'https://www.youtube.com/playlist?xlist=WL',
      'https://www.youtube.com/playlist?x=WL',
      'https://www.youtube.com/playlist?list=PL123',
      'https://www.youtube.com/playlist',
      'https://www.youtube.com/playlist/WL',
      'https://www.youtube.com/shorts/abc',
      'https://www.youtube.com/results?search_query=WL&list=WL',
      'https://www.youtube.com/@channel?list=WL',
      'https://music.youtube.com/playlist?list=WL',
      'https://youtube.com.evil.com/playlist?list=WL',
      'https://notyoutube.com/playlist?list=WL',
      'https://evil.com/playlist?list=WL',
      'javascript:alert(1)//youtube.com/playlist?list=WL',
      'not a url',
    ])
      expect(isWatchLaterUrl(url)).toBe(false);
  });
  it('blocks listed channels and study-mode surfaces by URL, never watch pages', () => {
    const block = {
      mode: 'blocklist',
      domains: [],
      youtube_blocked_channels: ['@blocked'],
    };
    expect(urlBlockedByPolicy('https://www.youtube.com/@blocked', block)).toBe(
      true,
    );
    expect(
      urlBlockedByPolicy('https://www.youtube.com/@blocked/videos', block),
    ).toBe(true);
    expect(urlBlockedByPolicy('https://www.youtube.com/@other', block)).toBe(
      false,
    );
    // Watch pages carry no channel: the content script decides, not this.
    expect(urlBlockedByPolicy('https://www.youtube.com/watch?v=x', block)).toBe(
      false,
    );
    const study = {
      mode: 'blocklist',
      domains: [],
      youtube_study_mode: true,
      youtube_allowed_channels: ['@allowed'],
    };
    expect(urlBlockedByPolicy('https://www.youtube.com/@allowed', study)).toBe(
      false,
    );
    expect(urlBlockedByPolicy('https://www.youtube.com/@other', study)).toBe(
      true,
    );
    expect(urlBlockedByPolicy('https://www.youtube.com/', study)).toBe(true);
    expect(
      urlBlockedByPolicy('https://www.youtube.com/shorts/abc', study),
    ).toBe(true);
    expect(
      urlBlockedByPolicy('https://www.youtube.com/feed/subscriptions', study),
    ).toBe(true);
    expect(
      urlBlockedByPolicy(
        'https://www.youtube.com/results?search_query=cats',
        study,
      ),
    ).toBe(true);
    expect(urlBlockedByPolicy('https://www.youtube.com/watch?v=x', study)).toBe(
      false,
    );
    expect(channelBlockReason('https://www.youtube.com/@other', study)).toBe(
      'study_channel',
    );
    expect(channelBlockReason('https://www.youtube.com/', study)).toBe(
      'study_surface',
    );
    expect(channelBlockReason('https://www.youtube.com/watch?v=x', study)).toBe(
      null,
    );
  });
  it('opens only the Watch Later playlist in both blocking modes', () => {
    const wl = 'https://www.youtube.com/playlist?list=WL';
    // Everything else on YouTube, including a video that carries
    // list=WL and playlists with other ids.
    const others = [
      'https://www.youtube.com/',
      'https://www.youtube.com/shorts/abc',
      'https://www.youtube.com/results?search_query=cats',
      'https://www.youtube.com/watch?v=x',
      'https://www.youtube.com/playlist?list=PL123',
      'https://www.youtube.com/playlist?list=WLxyz',
      'https://www.youtube.com/@blocked',
    ];
    // Mode 1: block distracting websites, youtube.com blocked.
    const block = {
      mode: 'blocklist',
      domains: ['youtube.com'],
      youtube_blocked_channels: ['@blocked'],
      youtube_allow_watch_later: true,
    };
    expect(urlBlockedByPolicy(wl, block)).toBe(false);
    for (const url of others) expect(urlBlockedByPolicy(url, block)).toBe(true);
    // Mode 2: allowlist that does NOT include youtube.com — the reported
    // bug: the Watch Later page must still open here.
    const allow = {
      mode: 'allowlist',
      domains: ['github.com', 'whatsapp.com', 'chatgpt.com'],
      youtube_allow_watch_later: true,
    };
    expect(urlBlockedByPolicy(wl, allow)).toBe(false);
    for (const url of others) expect(urlBlockedByPolicy(url, allow)).toBe(true);
    // A Watch Later *video* route opens at document level in both modes,
    // so the content script can verify membership. The URL itself is
    // never a permission: youtubeWatchDecision refuses the video when
    // the playlist does not contain it.
    const wlVideo = 'https://www.youtube.com/watch?v=x&list=WL';
    expect(urlBlockedByPolicy(wlVideo, block)).toBe(false);
    expect(urlBlockedByPolicy(wlVideo, allow)).toBe(false);
    expect(blockDetail(wlVideo, block)).toEqual({ blocked: false });
    // The toggle off, or a lookalike, is an ordinary blocked video.
    expect(
      urlBlockedByPolicy(wlVideo, {
        ...block,
        youtube_allow_watch_later: false,
      }),
    ).toBe(true);
    expect(
      urlBlockedByPolicy('https://www.youtube.com/watch?v=x&list=WLxyz', block),
    ).toBe(true);
    expect(
      urlBlockedByPolicy('https://www.youtube.com/watch?list=WL', block),
    ).toBe(true);
    // Toggle off: Watch Later is an ordinary blocked page in both modes.
    expect(
      urlBlockedByPolicy(wl, { ...block, youtube_allow_watch_later: false }),
    ).toBe(true);
    expect(
      urlBlockedByPolicy(wl, { ...allow, youtube_allow_watch_later: false }),
    ).toBe(true);
    // Offline still wins over Watch Later.
    expect(urlBlockedByPolicy(wl, { ...block, mode: 'offline' })).toBe(true);
    expect(urlBlockedByPolicy(wl, { ...allow, mode: 'offline' })).toBe(true);
    // The adult overlay still wins (its rule matches the host).
    expect(
      urlBlockedByPolicy(wl, { ...block, extra_blocked: ['youtube.com'] }),
    ).toBe(true);
    expect(
      urlBlockedByPolicy(wl, { ...allow, extra_blocked: ['youtube.com'] }),
    ).toBe(true);
    // An allowlisted youtube.com keeps behaving exactly as before.
    expect(urlBlockedByPolicy(wl, { ...allow, domains: ['youtube.com'] })).toBe(
      false,
    );
    expect(
      urlBlockedByPolicy('https://www.youtube.com/watch?v=x', {
        ...allow,
        domains: ['youtube.com'],
      }),
    ).toBe(false);
    // blockDetail agrees with urlBlockedByPolicy: no allowed route is
    // ever passed to the blocked page.
    expect(blockDetail(wl, allow)).toEqual({ blocked: false });
    expect(blockDetail('https://www.youtube.com/', allow)).toEqual({
      blocked: true,
      mechanism: 'allowlist',
      rule: null,
    });
    expect(
      blockDetail(wl, { ...allow, youtube_allow_watch_later: false }),
    ).toEqual({ blocked: true, mechanism: 'allowlist', rule: null });
  });
  it('lets exceptions override channel rules', () => {
    const policy = {
      mode: 'blocklist',
      domains: [],
      exceptions: ['youtube.com/@blocked'],
      youtube_blocked_channels: ['@blocked'],
    };
    expect(urlBlockedByPolicy('https://www.youtube.com/@blocked', policy)).toBe(
      false,
    );
    expect(
      urlBlockedByPolicy('https://www.youtube.com/@other', {
        ...policy,
        exceptions: ['youtube.com'],
        youtube_blocked_channels: ['@other'],
      }),
    ).toBe(false);
    expect(
      youtubeWatchDecision('https://www.youtube.com/watch?v=x', '@blocked', {
        mode: 'blocklist',
        domains: [],
        exceptions: ['youtube.com'],
        youtube_blocked_channels: ['@blocked'],
      }),
    ).toEqual({ blocked: false, reason: 'exception' });
  });
  it('decides watch pages fail-open in block mode and fail-closed in study mode', () => {
    const block = {
      mode: 'blocklist',
      domains: [],
      youtube_blocked_channels: ['@blocked'],
    };
    expect(youtubeWatchBlocked('@blocked', block)).toBe(true);
    expect(youtubeWatchBlocked('@other', block)).toBe(false);
    expect(youtubeWatchBlocked(null, block)).toBe(false);
    const study = {
      mode: 'blocklist',
      domains: [],
      youtube_study_mode: true,
      youtube_allowed_channels: ['@allowed'],
    };
    expect(youtubeWatchBlocked('@allowed', study)).toBe(false);
    expect(youtubeWatchBlocked('@other', study)).toBe(true);
    expect(youtubeWatchBlocked(null, study)).toBe(false);
    expect(youtubeWatchBlocked(null, study, true)).toBe(true);
    expect(
      youtubeWatchDecision(
        'https://www.youtube.com/watch?v=x',
        '@blocked',
        block,
      ),
    ).toEqual({ blocked: true, reason: 'blocked_channel' });
    expect(
      youtubeWatchDecision(
        'https://www.youtube.com/watch?v=x',
        null,
        study,
        true,
      ),
    ).toEqual({ blocked: true, reason: 'study_unidentified' });
    // A video URL carrying list=WL is a video, not the playlist
    // exception: the channel decision still applies.
    expect(
      youtubeWatchDecision(
        'https://www.youtube.com/watch?v=x&list=WL',
        '@blocked',
        {
          ...block,
          youtube_allow_watch_later: true,
        },
      ),
    ).toEqual({ blocked: true, reason: 'blocked_channel' });
    // …and in an allowlist without youtube.com the toggle off means the
    // video is blocked by the allowlist, list=WL or not.
    const allowWl = {
      mode: 'allowlist',
      domains: ['github.com'],
      youtube_allow_watch_later: true,
    };
    expect(
      youtubeWatchDecision('https://www.youtube.com/watch?v=x&list=WL', null, {
        ...allowWl,
        youtube_allow_watch_later: false,
      }),
    ).toEqual({ blocked: true, reason: 'domain' });
    // With the toggle on the route opens, but membership is unproven:
    // the decision stays pending instead of allowing playback.
    expect(
      youtubeWatchDecision(
        'https://www.youtube.com/watch?v=x&list=WL',
        null,
        allowWl,
      ),
    ).toEqual({ blocked: false, reason: null, pending: true });
    // Verified membership allows the video; an unverifiable one is blocked.
    expect(
      youtubeWatchDecision(
        'https://www.youtube.com/watch?v=x&list=WL',
        null,
        allowWl,
        false,
        true,
      ),
    ).toEqual({ blocked: false, reason: 'watch_later' });
    expect(
      youtubeWatchDecision(
        'https://www.youtube.com/watch?v=x&list=WL',
        null,
        allowWl,
        false,
        false,
      ),
    ).toEqual({ blocked: true, reason: 'watch_later_unverified' });
    // An explicitly blocked channel wins over verified Watch Later.
    expect(
      youtubeWatchDecision(
        'https://www.youtube.com/watch?v=x&list=WL',
        '@blocked',
        { ...allowWl, youtube_blocked_channels: ['@blocked'] },
        false,
        true,
      ),
    ).toEqual({ blocked: true, reason: 'blocked_channel' });
  });
  it('compiles channel DNR rules with the right priorities', () => {
    const block = compileRules({
      mode: 'blocklist',
      domains: [],
      youtube_blocked_channels: ['@blocked'],
    });
    const blockedRule = block.find(
      (r) => r.action.type === 'block' && r.condition.regexFilter,
    );
    expect(blockedRule).toBeDefined();
    expect(
      new RegExp(blockedRule.condition.regexFilter).test(
        'https://www.youtube.com/@blocked',
      ),
    ).toBe(true);
    expect(
      new RegExp(blockedRule.condition.regexFilter).test(
        'https://www.youtube.com/@BLOCKED/videos',
      ),
    ).toBe(true);
    expect(
      new RegExp(blockedRule.condition.regexFilter).test(
        'https://www.youtube.com/@other',
      ),
    ).toBe(false);
    expect(
      new RegExp(blockedRule.condition.regexFilter).test(
        'https://www.youtube.com/watch?v=x',
      ),
    ).toBe(false);
    const study = compileRules({
      mode: 'blocklist',
      domains: [],
      youtube_study_mode: true,
      youtube_allowed_channels: ['@allowed'],
      youtube_allow_watch_later: true,
    });
    const allows = study.filter((r) => r.action.type === 'allow');
    // Listed-channel allow (priority 2) and Watch-Later allow (priority 2).
    expect(
      allows.some(
        (r) =>
          r.priority === 2 &&
          r.condition.regexFilter &&
          new RegExp(r.condition.regexFilter).test(
            'https://www.youtube.com/@allowed',
          ),
      ),
    ).toBe(true);
    const wl = allows.find((r) =>
      /list=WL/.test(r.condition.regexFilter || ''),
    );
    expect(wl).toBeDefined();
    const wlMatches = (url) => new RegExp(wl.condition.regexFilter).test(url);
    expect(wlMatches('https://www.youtube.com/playlist?list=WL')).toBe(true);
    expect(wlMatches('https://m.youtube.com/playlist?index=2&list=WL')).toBe(
      true,
    );
    // A video carrying list=WL is not the playlist, so the route regex
    // must not open it.
    expect(wlMatches('https://www.youtube.com/watch?v=x&list=WL')).toBe(false);
    for (const url of [
      'https://www.youtube.com/playlist?list=WLxyz',
      'https://www.youtube.com/playlist?list=PL123',
      'https://www.youtube.com/playlist?xlist=WL',
      'https://www.youtube.com/',
      'https://www.youtube.com/shorts/abc',
    ])
      expect(wlMatches(url)).toBe(false);
    const surface = study.filter(
      (r) => r.action.type === 'block' && r.priority === 1,
    );
    expect(
      surface.some((r) =>
        new RegExp(r.condition.regexFilter).test('https://www.youtube.com/'),
      ),
    ).toBe(true);
    expect(
      surface.some((r) =>
        new RegExp(r.condition.regexFilter).test(
          'https://www.youtube.com/shorts/abc',
        ),
      ),
    ).toBe(true);
    expect(
      surface.some((r) =>
        new RegExp(r.condition.regexFilter).test(
          'https://www.youtube.com/results?search_query=cats',
        ),
      ),
    ).toBe(true);
    // In allowlist mode channel blocks outrank domain allows.
    const allow = compileRules({
      mode: 'allowlist',
      domains: ['youtube.com'],
      youtube_blocked_channels: ['@blocked'],
    });
    const channelBlock = allow.find(
      (r) =>
        r.action.type === 'block' &&
        r.priority === 4 &&
        r.condition.regexFilter,
    );
    expect(channelBlock).toBeDefined();
    expect(
      new RegExp(channelBlock.condition.regexFilter).test(
        'https://www.youtube.com/@blocked',
      ),
    ).toBe(true);
    expect(() =>
      compileRules({
        mode: 'blocklist',
        domains: [],
        youtube_blocked_channels: ['youtube.com/watch?v=x'],
      }),
    ).toThrow();
  });
  it('compiles the Watch Later allow above the allowlist catch-all', () => {
    const policy = {
      mode: 'allowlist',
      domains: ['github.com'],
      extra_blocked: ['pornhub.com'],
      youtube_allow_watch_later: true,
    };
    const rules = compileRules(policy);
    const wl = rules.find((r) => /list=WL/.test(r.condition.regexFilter || ''));
    expect(wl).toBeDefined();
    expect(wl.action.type).toBe('allow');
    // Above the catch-all block that closes youtube.com (priority 1)…
    const catchAll = rules.find(
      (r) => r.action.type === 'block' && r.priority === 1,
    );
    expect(catchAll).toBeDefined();
    expect(wl.priority).toBeGreaterThan(catchAll.priority);
    // …and below the adult overlay (priority 3), which still wins.
    const overlay = rules.find(
      (r) => r.action.type === 'block' && r.priority === 3,
    );
    expect(overlay).toBeDefined();
    expect(wl.priority).toBeLessThan(overlay.priority);
    const matches = (url) => new RegExp(wl.condition.regexFilter).test(url);
    expect(matches('https://www.youtube.com/playlist?list=WL')).toBe(true);
    expect(matches('https://www.youtube.com/')).toBe(false);
    expect(matches('https://www.youtube.com/watch?v=x&list=WL')).toBe(false);
    // With the toggle off there is no Watch Later rule at all.
    const off = compileRules({ ...policy, youtube_allow_watch_later: false });
    expect(off.some((r) => /list=WL/.test(r.condition.regexFilter || ''))).toBe(
      false,
    );
    // The blocklist mode keeps a matching allow rule.
    const blocklist = compileRules({
      mode: 'blocklist',
      domains: ['youtube.com'],
      youtube_allow_watch_later: true,
    });
    const blWl = blocklist.find((r) =>
      /list=WL/.test(r.condition.regexFilter || ''),
    );
    expect(blWl).toBeDefined();
    expect(blWl.action.type).toBe('allow');
    expect(blWl.priority).toBeGreaterThan(0);
  });
});

describe('blockDetail: which rule blocks a URL', () => {
  it('returns the matched extra_blocked rule for per-rule reasons', () => {
    const policy = {
      mode: 'blocklist',
      domains: ['example.com'],
      extra_blocked: ['youtube.com', '*.example.com'],
    };
    expect(blockDetail('https://www.youtube.com/watch?v=x', policy)).toEqual({
      blocked: true,
      mechanism: 'extra',
      rule: 'youtube.com',
    });
    expect(blockDetail('https://app.example.com/', policy)).toEqual({
      blocked: true,
      mechanism: 'extra',
      rule: '*.example.com',
    });
    // Session rules report no per-rule entry.
    expect(blockDetail('https://example.com/', policy)).toEqual({
      blocked: true,
      mechanism: 'domain',
      rule: null,
    });
    expect(blockDetail('https://other.com/', policy)).toEqual({
      blocked: false,
    });
  });
  it('mirrors urlBlockedByPolicy precedence: exceptions, keywords, allowlist, offline', () => {
    const policy = {
      mode: 'blocklist',
      domains: ['example.com'],
      keywords: ['chess'],
      exceptions: ['example.com'],
    };
    // Exception allows.
    expect(blockDetail('https://example.com/', policy)).toEqual({
      blocked: false,
    });
    // Keyword block.
    expect(
      blockDetail('https://www.google.com/search?q=chess', policy).mechanism,
    ).toBe('keyword');
    // Allowlist: unlisted is blocked with the session mechanism.
    const allow = { mode: 'allowlist', domains: ['example.com'] };
    expect(blockDetail('https://other.com/', allow)).toEqual({
      blocked: true,
      mechanism: 'allowlist',
      rule: null,
    });
    expect(blockDetail('https://example.com/', allow)).toEqual({
      blocked: false,
    });
    // Offline blocks everything.
    expect(
      blockDetail('https://anything.com/', { mode: 'offline' }).mechanism,
    ).toBe('offline');
    // Loopback and non-http(s) never block.
    expect(blockDetail('http://127.0.0.1:47832/x', policy)).toEqual({
      blocked: false,
    });
    expect(blockDetail('file:///etc/hosts', policy)).toEqual({
      blocked: false,
    });
    expect(blockDetail('not a url', policy)).toEqual({ blocked: false });
    expect(blockDetail('https://x.com/', null)).toEqual({ blocked: false });
  });
});

describe('youtubeVideoDecision Watch Later and blocked-channel precedence', () => {
  it('never authorizes a Watch Later video before channel identity is verified when channel blocks exist', () => {
    const policy = {
      mode: 'allowlist',
      domains: ['github.com'],
      youtube_allow_watch_later: true,
      youtube_blocked_channels: ['@blocked'],
    };
    const wlVideo = 'https://www.youtube.com/watch?v=abc123&list=WL';

    // Playlist membership verified early, but channel identity not yet known -> stays pending.
    expect(youtubeVideoDecision(wlVideo, [], policy, false, true)).toEqual({
      blocked: false,
      reason: null,
      pending: true,
    });
    // Detection window closes without channel identity -> fails closed.
    expect(youtubeVideoDecision(wlVideo, [], policy, true, true)).toEqual({
      blocked: true,
      reason: 'channel_unidentified',
    });
    // Blocked channel identity -> blocked even when in Watch Later.
    expect(
      youtubeVideoDecision(
        wlVideo,
        ['UCAllowedButAlso...', '@blocked'],
        policy,
        false,
        true,
      ),
    ).toEqual({
      blocked: true,
      reason: 'blocked_channel',
    });
    // Non-blocked identity + verified Watch Later membership -> allowed.
    expect(
      youtubeVideoDecision(wlVideo, ['@allowed'], policy, false, true),
    ).toEqual({
      blocked: false,
      reason: 'watch_later',
    });
    // Channel page URL itself is blocked immediately without waiting on identities.
    expect(
      youtubeVideoDecision(
        'https://www.youtube.com/@blocked',
        [],
        policy,
        false,
        null,
      ),
    ).toEqual({
      blocked: true,
      reason: 'blocked_channel',
    });
    // Watch Later playlist route itself is never held pending waiting for a video channel.
    expect(
      youtubeVideoDecision(
        'https://www.youtube.com/playlist?list=WL',
        [],
        policy,
        false,
        null,
      ),
    ).toEqual({
      blocked: false,
      reason: null,
    });
  });

  it('treats /@handle/live, /channel/UC.../live, /c/..., and /user/.../live as video routes and respects study-mode aliases', () => {
    expect(isChannelLivePath('/@veritasium/live')).toBe(true);
    expect(isChannelLivePath('/channel/UCHnyfMqiRRG1u-2MsSQLbXA/live')).toBe(
      true,
    );
    expect(isChannelLivePath('/c/veritasium/live')).toBe(true);
    expect(isChannelLivePath('/user/1veritasium/live')).toBe(true);
    expect(isChannelLivePath('/@veritasium/videos')).toBe(false);
    expect(isChannelLivePath('/@veritasium')).toBe(false);

    const blockPolicy = {
      mode: 'blocklist',
      domains: [],
      youtube_blocked_channels: ['UCHnyfMqiRRG1u-2MsSQLbXA'],
    };
    expect(
      youtubeVideoDecision(
        'https://www.youtube.com/@other/live',
        [],
        blockPolicy,
        false,
        null,
      ),
    ).toEqual({ blocked: false, reason: null, pending: true });
    expect(
      youtubeVideoDecision(
        'https://www.youtube.com/@other/live',
        [],
        blockPolicy,
        true,
        null,
      ),
    ).toEqual({ blocked: true, reason: 'channel_unidentified' });
    expect(
      youtubeVideoDecision(
        'https://www.youtube.com/@veritasium/live',
        ['@veritasium', 'UCHnyfMqiRRG1u-2MsSQLbXA'],
        blockPolicy,
        false,
        null,
      ),
    ).toEqual({ blocked: true, reason: 'blocked_channel' });

    const studyPolicy = {
      mode: 'blocklist',
      domains: [],
      youtube_study_mode: true,
      youtube_allowed_channels: ['UCAllowedId123456789012'],
      youtube_blocked_channels: ['UCBlockedId123456789012'],
      youtube_allow_watch_later: true,
    };
    // Alias keys allow /@allowed/live when its UC ID is allowlisted.
    expect(
      youtubeVideoDecision(
        'https://www.youtube.com/@allowed/live',
        ['@allowed', 'UCAllowedId123456789012'],
        studyPolicy,
        false,
        null,
      ),
    ).toEqual({ blocked: false, reason: null });
    // Explicit block wins over allowed + Watch Later in Study Mode.
    expect(
      youtubeVideoDecision(
        'https://www.youtube.com/watch?v=x&list=WL',
        ['UCAllowedId123456789012', 'UCBlockedId123456789012'],
        studyPolicy,
        false,
        true,
      ),
    ).toEqual({ blocked: true, reason: 'blocked_channel' });
  });

  it('determines Watch Later homepage/feed redirects accurately without redirecting offline, adult overlay, or music hosts', () => {
    const allowWl = {
      mode: 'allowlist',
      domains: ['github.com'],
      youtube_allow_watch_later: true,
    };
    expect(watchLaterOnly(allowWl)).toBe(true);
    expect(
      shouldRedirectToWatchLater('https://www.youtube.com/', allowWl),
    ).toBe(true);
    expect(
      shouldRedirectToWatchLater(
        'https://www.youtube.com/feed/subscriptions',
        allowWl,
      ),
    ).toBe(true);
    expect(watchLaterRedirectUrl('https://www.youtube.com/', allowWl)).toBe(
      'https://www.youtube.com/playlist?list=WL',
    );
    expect(watchLaterRedirectUrl('https://m.youtube.com/', allowWl)).toBe(
      'https://m.youtube.com/playlist?list=WL',
    );
    expect(
      shouldRedirectToWatchLater(
        'https://www.youtube.com/playlist?list=WL',
        allowWl,
      ),
    ).toBe(false);
    expect(
      shouldRedirectToWatchLater('https://music.youtube.com/', allowWl),
    ).toBe(false);

    // Study mode + Watch Later also redirects / -> /playlist?list=WL.
    const studyWl = {
      mode: 'blocklist',
      domains: [],
      youtube_study_mode: true,
      youtube_allow_watch_later: true,
    };
    expect(watchLaterRedirectUrl('https://www.youtube.com/', studyWl)).toBe(
      'https://www.youtube.com/playlist?list=WL',
    );

    // Offline mode or desktop adult overlay over youtube.com never redirects to WL.
    expect(watchLaterOnly({ ...allowWl, mode: 'offline' })).toBe(false);
    expect(
      watchLaterRedirectUrl('https://www.youtube.com/', {
        ...allowWl,
        mode: 'offline',
      }),
    ).toBe(null);
    expect(watchLaterOnly({ ...allowWl, extra_blocked: ['youtube.com'] })).toBe(
      false,
    );
  });
});
