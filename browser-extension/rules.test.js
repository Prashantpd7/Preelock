import { describe, it, expect } from 'vitest';
import {
  blockDetail,
  channelBlockReason,
  channelKeyFromPath,
  compileRules,
  extractChannelKey,
  isWatchLaterUrl,
  leaseValid,
  normalizeYouTubeChannel,
  ruleMatches,
  searchPageQuery,
  urlBlockedByPolicy,
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
    expect(ruleMatches('youtube.com/shorts', 'youtube.com', '/shorts')).toBe(true);
    expect(ruleMatches('youtube.com/shorts', 'youtube.com', '/shorts/abc')).toBe(true);
    expect(ruleMatches('youtube.com/shorts', 'youtube.com', '/shortsabc')).toBe(false);
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
      urlBlockedByPolicy('https://www.google.com/search?q=chess+openings', policy),
    ).toBe(true);
    expect(
      urlBlockedByPolicy('https://www.google.com/search?q=checkers', policy),
    ).toBe(false);
    expect(
      urlBlockedByPolicy('https://www.youtube.com/results?search_query=chess', policy),
    ).toBe(true);
    expect(
      urlBlockedByPolicy('https://www.youtube.com/watch?v=abc', policy),
    ).toBe(false);
    expect(urlBlockedByPolicy('https://pornhub.com/', policy)).toBe(true);
    expect(searchPageQuery('https://www.bing.com/search?q=chess')?.query).toBe('chess');
    expect(searchPageQuery('https://duckduckgo.com/?q=chess')?.query).toBe('chess');
    expect(searchPageQuery('https://duckduckgo.com/?q=chess&ia=web')?.query).toBe('chess');
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
    expect(urlBlockedByPolicy('https://www.youtube.com/watch?v=x', policy)).toBe(false);
    // A path exception allows only that path, not the whole domain.
    expect(urlBlockedByPolicy('https://example.com/docs/intro', policy)).toBe(false);
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
    expect(urlBlockedByPolicy('file:///Users/x/notes.html', policy)).toBe(false);
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
      overlap.filter(
        (r) => r.action.type === 'block' && r.priority === 3,
      ),
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
    expect(normalizeYouTubeChannel('https://www.youtube.com/@SomeHandle/')).toBe('@somehandle');
    expect(normalizeYouTubeChannel('youtube.com/@handle/videos')).toBe('@handle');
    expect(normalizeYouTubeChannel('youtube.com/channel/UCabcdefghijklmnopqrstuv')).toBe('UCabcdefghijklmnopqrstuv');
    expect(normalizeYouTubeChannel('UCabcdefghijklmnopqrstuv')).toBe('UCabcdefghijklmnopqrstuv');
    expect(normalizeYouTubeChannel('https://youtube.com/c/SomeName')).toBe('c:somename');
    expect(normalizeYouTubeChannel('youtube.com/user/SomeName')).toBe('user:somename');
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
    expect(channelKeyFromPath('/channel/UCabcdefghijklmnopqrstuv')).toBe('UCabcdefghijklmnopqrstuv');
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
    expect(extractChannelKey(['/watch?v=x', 'https://example.com/@x'])).toBe(null);
    expect(extractChannelKey([])).toBe(null);
  });
  it('recognizes Watch-Later URLs only', () => {
    expect(isWatchLaterUrl('https://www.youtube.com/watch?v=x&list=WL')).toBe(true);
    expect(isWatchLaterUrl('https://www.youtube.com/playlist?list=WL')).toBe(true);
    expect(isWatchLaterUrl('https://www.youtube.com/watch?v=x&list=WLxyz')).toBe(false);
    expect(isWatchLaterUrl('https://www.youtube.com/watch?v=x')).toBe(false);
    expect(isWatchLaterUrl('https://www.youtube.com/playlist?list=PL123')).toBe(false);
    expect(isWatchLaterUrl('not a url')).toBe(false);
  });
  it('blocks listed channels and study-mode surfaces by URL, never watch pages', () => {
    const block = {
      mode: 'blocklist',
      domains: [],
      youtube_blocked_channels: ['@blocked'],
    };
    expect(urlBlockedByPolicy('https://www.youtube.com/@blocked', block)).toBe(true);
    expect(urlBlockedByPolicy('https://www.youtube.com/@blocked/videos', block)).toBe(true);
    expect(urlBlockedByPolicy('https://www.youtube.com/@other', block)).toBe(false);
    // Watch pages carry no channel: the content script decides, not this.
    expect(urlBlockedByPolicy('https://www.youtube.com/watch?v=x', block)).toBe(false);
    const study = {
      mode: 'blocklist',
      domains: [],
      youtube_study_mode: true,
      youtube_allowed_channels: ['@allowed'],
    };
    expect(urlBlockedByPolicy('https://www.youtube.com/@allowed', study)).toBe(false);
    expect(urlBlockedByPolicy('https://www.youtube.com/@other', study)).toBe(true);
    expect(urlBlockedByPolicy('https://www.youtube.com/', study)).toBe(true);
    expect(urlBlockedByPolicy('https://www.youtube.com/shorts/abc', study)).toBe(true);
    expect(urlBlockedByPolicy('https://www.youtube.com/feed/subscriptions', study)).toBe(true);
    expect(urlBlockedByPolicy('https://www.youtube.com/results?search_query=cats', study)).toBe(true);
    expect(urlBlockedByPolicy('https://www.youtube.com/watch?v=x', study)).toBe(false);
    expect(channelBlockReason('https://www.youtube.com/@other', study)).toBe('study_channel');
    expect(channelBlockReason('https://www.youtube.com/', study)).toBe('study_surface');
    expect(channelBlockReason('https://www.youtube.com/watch?v=x', study)).toBe(null);
  });
  it('lets Watch Later through a domain block and past channel rules', () => {
    const policy = {
      mode: 'blocklist',
      domains: ['youtube.com'],
      youtube_blocked_channels: ['@blocked'],
      youtube_allow_watch_later: true,
    };
    expect(urlBlockedByPolicy('https://www.youtube.com/watch?v=x&list=WL', policy)).toBe(false);
    expect(urlBlockedByPolicy('https://www.youtube.com/playlist?list=WL', policy)).toBe(false);
    expect(urlBlockedByPolicy('https://www.youtube.com/watch?v=x', policy)).toBe(true);
    // Without the toggle, Watch Later is just another blocked page.
    expect(
      urlBlockedByPolicy('https://www.youtube.com/watch?v=x&list=WL', {
        ...policy,
        youtube_allow_watch_later: false,
      }),
    ).toBe(true);
    // Offline still wins over Watch Later.
    expect(
      urlBlockedByPolicy('https://www.youtube.com/watch?v=x&list=WL', {
        ...policy,
        mode: 'offline',
      }),
    ).toBe(true);
    // In allowlist mode a non-listed youtube.com stays blocked even for WL,
    // but a listed one bypasses channel rules for WL pages.
    const allow = {
      mode: 'allowlist',
      domains: ['docs.rs'],
      youtube_allow_watch_later: true,
    };
    expect(urlBlockedByPolicy('https://www.youtube.com/watch?v=x&list=WL', allow)).toBe(true);
    expect(
      urlBlockedByPolicy('https://www.youtube.com/watch?v=x&list=WL', {
        ...allow,
        domains: ['youtube.com'],
        youtube_study_mode: true,
        youtube_allowed_channels: ['@allowed'],
      }),
    ).toBe(false);
  });
  it('lets exceptions override channel rules', () => {
    const policy = {
      mode: 'blocklist',
      domains: [],
      exceptions: ['youtube.com/@blocked'],
      youtube_blocked_channels: ['@blocked'],
    };
    expect(urlBlockedByPolicy('https://www.youtube.com/@blocked', policy)).toBe(false);
    expect(urlBlockedByPolicy('https://www.youtube.com/@other', {
      ...policy,
      exceptions: ['youtube.com'],
      youtube_blocked_channels: ['@other'],
    })).toBe(false);
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
    const block = { mode: 'blocklist', domains: [], youtube_blocked_channels: ['@blocked'] };
    expect(youtubeWatchBlocked('@blocked', block)).toBe(true);
    expect(youtubeWatchBlocked('@other', block)).toBe(false);
    expect(youtubeWatchBlocked(null, block)).toBe(false);
    const study = { mode: 'blocklist', domains: [], youtube_study_mode: true, youtube_allowed_channels: ['@allowed'] };
    expect(youtubeWatchBlocked('@allowed', study)).toBe(false);
    expect(youtubeWatchBlocked('@other', study)).toBe(true);
    expect(youtubeWatchBlocked(null, study)).toBe(false);
    expect(youtubeWatchBlocked(null, study, true)).toBe(true);
    expect(
      youtubeWatchDecision('https://www.youtube.com/watch?v=x', '@blocked', block),
    ).toEqual({ blocked: true, reason: 'blocked_channel' });
    expect(
      youtubeWatchDecision('https://www.youtube.com/watch?v=x', null, study, true),
    ).toEqual({ blocked: true, reason: 'study_unidentified' });
    expect(
      youtubeWatchDecision('https://www.youtube.com/watch?v=x&list=WL', '@blocked', {
        ...block,
        youtube_allow_watch_later: true,
      }),
    ).toEqual({ blocked: false, reason: 'watch_later' });
  });
  it('compiles channel DNR rules with the right priorities', () => {
    const block = compileRules({
      mode: 'blocklist',
      domains: [],
      youtube_blocked_channels: ['@blocked'],
    });
    const blockedRule = block.find((r) => r.action.type === 'block' && r.condition.regexFilter);
    expect(blockedRule).toBeDefined();
    expect(new RegExp(blockedRule.condition.regexFilter).test('https://www.youtube.com/@blocked')).toBe(true);
    expect(new RegExp(blockedRule.condition.regexFilter).test('https://www.youtube.com/@BLOCKED/videos')).toBe(true);
    expect(new RegExp(blockedRule.condition.regexFilter).test('https://www.youtube.com/@other')).toBe(false);
    expect(new RegExp(blockedRule.condition.regexFilter).test('https://www.youtube.com/watch?v=x')).toBe(false);
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
          new RegExp(r.condition.regexFilter).test('https://www.youtube.com/@allowed'),
      ),
    ).toBe(true);
    const wl = allows.find((r) => /list=WL/.test(r.condition.regexFilter || ''));
    expect(wl).toBeDefined();
    expect(new RegExp(wl.condition.regexFilter).test('https://www.youtube.com/watch?v=x&list=WL')).toBe(true);
    expect(new RegExp(wl.condition.regexFilter).test('https://www.youtube.com/watch?v=x&list=WLxyz')).toBe(false);
    expect(new RegExp(wl.condition.regexFilter).test('https://www.youtube.com/watch?v=x')).toBe(false);
    const surface = study.filter((r) => r.action.type === 'block' && r.priority === 1);
    expect(surface.some((r) => new RegExp(r.condition.regexFilter).test('https://www.youtube.com/'))).toBe(true);
    expect(surface.some((r) => new RegExp(r.condition.regexFilter).test('https://www.youtube.com/shorts/abc'))).toBe(true);
    expect(surface.some((r) => new RegExp(r.condition.regexFilter).test('https://www.youtube.com/results?search_query=cats'))).toBe(true);
    // In allowlist mode channel blocks outrank domain allows.
    const allow = compileRules({
      mode: 'allowlist',
      domains: ['youtube.com'],
      youtube_blocked_channels: ['@blocked'],
    });
    const channelBlock = allow.find(
      (r) => r.action.type === 'block' && r.priority === 4 && r.condition.regexFilter,
    );
    expect(channelBlock).toBeDefined();
    expect(new RegExp(channelBlock.condition.regexFilter).test('https://www.youtube.com/@blocked')).toBe(true);
    expect(() =>
      compileRules({ mode: 'blocklist', domains: [], youtube_blocked_channels: ['youtube.com/watch?v=x'] }),
    ).toThrow();
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
    expect(blockDetail('https://other.com/', policy)).toEqual({ blocked: false });
  });
  it('mirrors urlBlockedByPolicy precedence: exceptions, keywords, allowlist, offline', () => {
    const policy = {
      mode: 'blocklist',
      domains: ['example.com'],
      keywords: ['chess'],
      exceptions: ['example.com'],
    };
    // Exception allows.
    expect(blockDetail('https://example.com/', policy)).toEqual({ blocked: false });
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
    expect(blockDetail('https://example.com/', allow)).toEqual({ blocked: false });
    // Offline blocks everything.
    expect(
      blockDetail('https://anything.com/', { mode: 'offline' }).mechanism,
    ).toBe('offline');
    // Loopback and non-http(s) never block.
    expect(blockDetail('http://127.0.0.1:47832/x', policy)).toEqual({
      blocked: false,
    });
    expect(blockDetail('file:///etc/hosts', policy)).toEqual({ blocked: false });
    expect(blockDetail('not a url', policy)).toEqual({ blocked: false });
    expect(blockDetail('https://x.com/', null)).toEqual({ blocked: false });
  });
});
