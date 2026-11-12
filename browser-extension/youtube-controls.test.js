import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const here = import.meta.url.startsWith('file:')
  ? path.dirname(fileURLToPath(import.meta.url))
  : path.join(process.cwd(), 'browser-extension');
const controlsSource = readFileSync(
  path.join(here, 'youtube-controls.js'),
  'utf8',
);
const sandbox = { URL };
sandbox.globalThis = sandbox;
vm.createContext(sandbox);
vm.runInContext(controlsSource, sandbox);
const controlsApi = sandbox.__preelockYoutubeControls;

const {
  STORAGE_KEY,
  SCHEMA_VERSION,
  CONTROLS_STYLE_ID,
  DEFAULT_YOUTUBE_CONTROLS,
  YOUTUBE_CONTROL_GROUPS,
  normalizeYoutubeControls,
  getControlValue,
  setControlValue,
  hasActiveYoutubeControls,
  countActiveControls,
  isMainYouTubeHost,
  isShortsPath,
  youtubeRouteKind,
  videoIdFromVideoUrl,
  controlsHiddenSelectors,
  controlsCss,
} = controlsApi;

describe('youtube-controls schema and normalization', () => {
  it('defines all 5 popup groups and initializes all controls to false', () => {
    expect(STORAGE_KEY).toBe('youtubeControls');
    expect(SCHEMA_VERSION).toBe(1);
    expect(CONTROLS_STYLE_ID).toBe('preelock-yt-controls-style');
    expect(YOUTUBE_CONTROL_GROUPS.map((g) => g.title)).toEqual([
      'Home and Navigation',
      'Video Recommendations',
      'Video Player',
      'Social and Community',
      'Other YouTube Elements',
    ]);
    const defaults = normalizeYoutubeControls(null);
    expect(defaults).toEqual(DEFAULT_YOUTUBE_CONTROLS);
    expect(hasActiveYoutubeControls(defaults)).toBe(false);
    expect(countActiveControls(defaults)).toBe(0);
  });

  it('safely normalizes missing, corrupt, and partial stored settings', () => {
    expect(normalizeYoutubeControls(undefined)).toEqual(
      DEFAULT_YOUTUBE_CONTROLS,
    );
    expect(normalizeYoutubeControls('bad')).toEqual(DEFAULT_YOUTUBE_CONTROLS);
    expect(normalizeYoutubeControls([])).toEqual(DEFAULT_YOUTUBE_CONTROLS);
    const partial = normalizeYoutubeControls({
      version: 99,
      home: { hideHomeFeed: true, redirectToSubscriptions: 'yes', extra: true },
      navigation: null,
      player: { disableAutoplay: 1, disableAnnotations: true },
      unknownSection: { foo: true },
    });
    expect(partial.version).toBe(SCHEMA_VERSION);
    expect(partial.home.hideHomeFeed).toBe(true);
    expect(partial.home.redirectToSubscriptions).toBe(false);
    expect(partial.home.extra).toBeUndefined();
    expect(partial.player.disableAutoplay).toBe(false);
    expect(partial.player.disableAnnotations).toBe(true);
    expect(partial.unknownSection).toBeUndefined();
  });

  it('updates controls immutably and counts unique canonical active controls', () => {
    const base = normalizeYoutubeControls(null);
    const next = setControlValue(base, 'social.hideNotifications', true);
    const next2 = setControlValue(next, 'recommendations.hideLiveChat', true);
    expect(getControlValue(base, 'social.hideNotifications')).toBe(false);
    expect(getControlValue(next2, 'social.hideNotifications')).toBe(true);
    expect(getControlValue(next2, 'recommendations.hideLiveChat')).toBe(true);
    expect(hasActiveYoutubeControls(next2)).toBe(true);
    // Live Chat (sidebar + social) and Notifications (social + top header)
    // each count once toward canonical active controls.
    expect(countActiveControls(next2)).toBe(2);
  });

  it('keeps Redirect to Subscriptions and Hide Subscriptions mutually exclusive', () => {
    const redirectOnly = setControlValue(
      null,
      'home.redirectToSubscriptions',
      true,
    );
    const thenHide = setControlValue(
      redirectOnly,
      'navigation.hideSubscriptions',
      true,
    );
    expect(thenHide.home.redirectToSubscriptions).toBe(false);
    expect(thenHide.navigation.hideSubscriptions).toBe(true);

    const thenRedirect = setControlValue(
      thenHide,
      'home.redirectToSubscriptions',
      true,
    );
    expect(thenRedirect.home.redirectToSubscriptions).toBe(true);
    expect(thenRedirect.navigation.hideSubscriptions).toBe(false);

    const normalizedBoth = normalizeYoutubeControls({
      home: { redirectToSubscriptions: true },
      navigation: { hideSubscriptions: true },
    });
    expect(normalizedBoth.home.redirectToSubscriptions).toBe(true);
    expect(normalizedBoth.navigation.hideSubscriptions).toBe(false);
  });
});

describe('youtube-controls URL and route helpers', () => {
  it('recognizes main YouTube hosts and rejects lookalikes or unrelated surfaces', () => {
    expect(isMainYouTubeHost('youtube.com')).toBe(true);
    expect(isMainYouTubeHost('www.youtube.com')).toBe(true);
    expect(isMainYouTubeHost('m.youtube.com')).toBe(true);
    expect(isMainYouTubeHost('music.youtube.com')).toBe(false);
    expect(isMainYouTubeHost('studio.youtube.com')).toBe(false);
    expect(isMainYouTubeHost('youtube.com.evil.com')).toBe(false);
  });

  it('classifies YouTube routes accurately', () => {
    expect(youtubeRouteKind('https://www.youtube.com/')).toBe('home');
    expect(youtubeRouteKind('https://www.youtube.com/feed/subscriptions')).toBe(
      'subscriptions',
    );
    expect(
      youtubeRouteKind('https://www.youtube.com/watch?v=dQw4w9WgXcQ'),
    ).toBe('watch');
    expect(youtubeRouteKind('https://www.youtube.com/shorts/abc12345')).toBe(
      'shorts',
    );
    expect(youtubeRouteKind('https://www.youtube.com/live/abc12345')).toBe(
      'live',
    );
    expect(
      youtubeRouteKind('https://www.youtube.com/results?search_query=ts'),
    ).toBe('search');
    expect(youtubeRouteKind('https://www.youtube.com/playlist?list=WL')).toBe(
      'playlist',
    );
    expect(youtubeRouteKind('https://www.youtube.com/@mkbhd/videos')).toBe(
      'channel',
    );
    expect(
      youtubeRouteKind(
        'https://www.youtube.com/channel/UCBJycsmduvYEL83R_U4JriQ',
      ),
    ).toBe('channel');
    expect(isShortsPath('/shorts')).toBe(true);
    expect(isShortsPath('/shorts/abc')).toBe(true);
    expect(isShortsPath('/watch')).toBe(false);
  });

  it('extracts video IDs across watch, shorts, and live URLs only', () => {
    expect(
      videoIdFromVideoUrl('https://www.youtube.com/watch?v=abc12345_X'),
    ).toBe('abc12345_X');
    expect(
      videoIdFromVideoUrl('https://www.youtube.com/shorts/shortId123'),
    ).toBe('shortId123');
    expect(videoIdFromVideoUrl('https://www.youtube.com/live/liveId_999')).toBe(
      'liveId_999',
    );
    expect(videoIdFromVideoUrl('https://www.youtube.com/@handle')).toBe(null);
    expect(videoIdFromVideoUrl('https://example.com/watch?v=abc12345_X')).toBe(
      null,
    );
  });
});

describe('youtube-controls individual CSS selectors and independence', () => {
  it('emits empty CSS when every control is disabled', () => {
    expect(controlsHiddenSelectors(null)).toEqual([]);
    expect(controlsCss(null)).toBe('');
  });

  it('scopes Hide Home Feed strictly to home route without hiding Subscriptions', () => {
    const cfg = setControlValue(null, 'home.hideHomeFeed', true);
    const selectors = controlsHiddenSelectors(cfg);
    expect(selectors.length).toBeGreaterThan(0);
    expect(
      selectors.every((s) => s.includes('data-preelock-route="home"')),
    ).toBe(true);
    expect(selectors).toContain(
      'html[data-preelock-route="home"] ytd-browse[page-subtype="home"] ytd-rich-grid-renderer',
    );
  });

  it('emits no hiding CSS for Redirect to Subscriptions alone', () => {
    const cfg = setControlValue(null, 'home.redirectToSubscriptions', true);
    expect(controlsCss(cfg)).toBe('');
  });

  it('hides Explore/Trending without breaking Home, Subscriptions, or Watch Later nav', () => {
    const cfg = setControlValue(
      null,
      'navigation.hideExploreAndTrending',
      true,
    );
    const selectors = controlsHiddenSelectors(cfg);
    expect(selectors).toContain(
      'ytd-guide-entry-renderer:has(a[href^="/feed/trending"])',
    );
    expect(selectors).toContain(
      'ytd-guide-entry-renderer:has(a[href^="/feed/explore"])',
    );
    expect(selectors).toContain('[data-preelock-hidden="explore"]');
    const sectionRules = selectors.filter((s) =>
      s.startsWith('ytd-guide-section-renderer'),
    );
    expect(sectionRules.length).toBeGreaterThan(0);
    for (const rule of sectionRules) {
      expect(rule).toContain('not(:has(a[href="/playlist?list=WL"]))');
      expect(rule).toContain('not(:has(a[href="/feed/subscriptions"]))');
    }
  });

  it('hides Subscriptions navigation entry and subscribed-channels sidebar list independently without blanket #newness-dot hiding', () => {
    const cfg = setControlValue(null, 'navigation.hideSubscriptions', true);
    const selectors = controlsHiddenSelectors(cfg);
    expect(selectors).toContain('ytd-guide-subscriptions-section-renderer');
    expect(selectors).toContain(
      'ytd-guide-collapsible-section-entry-renderer:has(a[href="/feed/subscriptions"])',
    );
    expect(selectors).toContain(
      'ytd-guide-entry-renderer:has(a[href="/feed/subscriptions"])',
    );
    expect(selectors).toContain(
      'ytd-mini-guide-entry-renderer:has(a[href="/feed/subscriptions"])',
    );
    expect(selectors).toContain(
      'ytd-guide-entry-renderer:has(a[href="/feed/channels"])',
    );
    expect(selectors).toContain(
      'ytd-guide-entry-renderer[line-end-style="dot"]',
    );
    expect(
      selectors.some((s) =>
        s.includes('ytd-guide-section-renderer:has(a[href^="/@"])'),
      ),
    ).toBe(true);
    expect(selectors).not.toContain(
      'ytd-guide-entry-renderer:has(#newness-dot)',
    );
    expect(selectors).toContain('[data-preelock-hidden="subscriptions"]');
  });

  it('supports Hide Video Sidebar and each of its 4 sub-controls independently even when parent is off', () => {
    const parentOnly = setControlValue(
      null,
      'recommendations.hideVideoSidebar',
      true,
    );
    expect(controlsHiddenSelectors(parentOnly)).toContain(
      'ytd-watch-flexy #secondary',
    );

    const subExpectations = [
      [
        'recommendations.hideRecommended',
        'ytd-watch-next-secondary-results-renderer',
      ],
      ['recommendations.hideLiveChat', 'ytd-live-chat-frame'],
      [
        'recommendations.hidePlaylist',
        'ytd-watch-flexy ytd-playlist-panel-renderer',
      ],
      ['recommendations.hideFundraiser', 'ytd-donation-shelf-renderer'],
    ];
    for (const [path, expectedSelector] of subExpectations) {
      const onlySub = setControlValue(null, path, true);
      expect(getControlValue(onlySub, 'recommendations.hideVideoSidebar')).toBe(
        false,
      );
      const selectors = controlsHiddenSelectors(onlySub);
      expect(selectors).not.toContain('ytd-watch-flexy #secondary');
      expect(selectors).toContain(expectedSelector);
    }
  });

  it('generates selectors for End Screen Feed, End Screen Cards, Shorts, and Mixes', () => {
    const endFeed = controlsHiddenSelectors(
      setControlValue(null, 'recommendations.hideEndScreenFeed', true),
    );
    expect(endFeed).toContain('ytd-watch-flexy .ytp-endscreen-content');
    expect(endFeed).toContain('.ytp-videowall-still');

    const endCards = controlsHiddenSelectors(
      setControlValue(null, 'recommendations.hideEndScreenCards', true),
    );
    expect(endCards).toContain('.ytp-ce-element');
    expect(endCards).toContain('.ytp-ce-covering-overlay');

    const shortsCfg = setControlValue(null, 'recommendations.hideShorts', true);
    const shortsSelectors = controlsHiddenSelectors(shortsCfg);
    expect(shortsSelectors).toContain(
      'ytd-guide-entry-renderer:has(a[href^="/shorts"])',
    );
    expect(shortsSelectors).toContain('ytd-reel-shelf-renderer');
    expect(shortsSelectors).toContain('ytd-rich-shelf-renderer[is-shorts]');
    expect(shortsSelectors).toContain(
      'ytd-video-renderer:has(a[href^="/shorts/"])',
    );
    expect(shortsSelectors).toContain('yt-tab-shape[tab-title="Shorts"]');
    expect(shortsSelectors).toContain('[data-preelock-hidden="shorts"]');
    expect(shortsSelectors).toContain(
      'html[data-preelock-route="shorts"] ytd-shorts',
    );
    expect(controlsCss(shortsCfg)).toContain('#preelock-shorts-cover');

    const mixes = controlsHiddenSelectors(
      setControlValue(null, 'recommendations.hideMixes', true),
    );
    expect(mixes).toContain('ytd-radio-renderer');
    expect(mixes).toContain('ytd-compact-radio-renderer');
    expect(mixes).toContain(
      'ytd-compact-video-renderer:has(a[href*="&list=RD"])',
    );
    expect(mixes).toContain('[data-preelock-hidden="mix"]');
  });

  it('supports Hide Video Info and its 4 sub-controls independently, plus Disable Autoplay and Disable Annotations', () => {
    const parentInfo = controlsHiddenSelectors(
      setControlValue(null, 'player.hideVideoInfo', true),
    );
    expect(parentInfo).toContain('ytd-watch-flexy #above-the-fold');
    expect(parentInfo).toContain('ytd-watch-flexy ytd-watch-metadata');

    const infoSubs = [
      ['player.hideDescription', 'ytd-watch-flexy #description'],
      ['player.hideMetadata', 'ytd-watch-flexy ytd-watch-info-text'],
      [
        'player.hideActionButtons',
        'ytd-watch-flexy ytd-watch-metadata #actions',
      ],
      ['player.hideChannelInfo', 'ytd-watch-flexy ytd-video-owner-renderer'],
    ];
    for (const [path, expected] of infoSubs) {
      const selectors = controlsHiddenSelectors(
        setControlValue(null, path, true),
      );
      expect(selectors).not.toContain('ytd-watch-flexy #above-the-fold');
      expect(selectors).toContain(expected);
    }

    const autoplay = controlsHiddenSelectors(
      setControlValue(null, 'player.disableAutoplay', true),
    );
    expect(autoplay).toContain('.ytp-autonav-endscreen-upnext-container');

    const annotations = controlsHiddenSelectors(
      setControlValue(null, 'player.disableAnnotations', true),
    );
    expect(annotations).toContain('.ytp-cards-teaser');
    expect(annotations).toContain('.iv-drawer');
    expect(annotations).not.toContain('.ytp-play-button');
    expect(annotations).not.toContain('.ytp-progress-bar');
  });

  it('supports Hide Comments + sub-controls independently, Hide Notifications, Hide Merch, and Hide Top Header layout rules', () => {
    const comments = controlsHiddenSelectors(
      setControlValue(null, 'social.hideComments', true),
    );
    expect(comments).toContain('ytd-comments');
    expect(comments).toContain('#comments');

    const teaserOnly = controlsHiddenSelectors(
      setControlValue(null, 'social.hideCommentTeaser', true),
    );
    expect(teaserOnly).toContain('#comment-teaser');
    expect(teaserOnly).not.toContain('ytd-comments');

    const repliesOnly = controlsHiddenSelectors(
      setControlValue(null, 'social.hideCommentReplies', true),
    );
    expect(repliesOnly).toContain('ytd-comment-replies-renderer');
    expect(repliesOnly).not.toContain('ytd-comments');

    const notif = controlsHiddenSelectors(
      setControlValue(null, 'social.hideNotifications', true),
    );
    expect(notif).toContain('ytd-notification-topbar-button-renderer');

    const merch = controlsHiddenSelectors(
      setControlValue(null, 'other.hideMerchTicketsOffers', true),
    );
    expect(merch).toContain('ytd-merch-shelf-renderer');
    expect(merch).toContain('ytd-ticket-shelf-renderer');
    expect(merch).toContain('ytd-offer-module-renderer');

    const headerCfg = setControlValue(null, 'other.hideTopHeader', true);
    const headerCss = controlsCss(headerCfg);
    expect(headerCss).toContain('#masthead-container');
    expect(headerCss).toContain('--ytd-masthead-height:0px!important');
    expect(headerCss).toContain('ytd-mini-guide-renderer{top:0!important}');
  });
});
