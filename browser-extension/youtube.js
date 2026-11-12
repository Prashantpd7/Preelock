// YouTube enforcement (session-scoped blocking + user YouTube Controls).
//
// Two distinct layers coexist cleanly:
// 1. Authoritative session policy (from the desktop app via worker.js):
//    website/surface rules, channel blocking, study mode, and the Watch
//    Later playlist exception. Active ONLY while a session policy lease
//    is valid; all session UI/guards disappear when the session ends.
// 2. Persistent user YouTube Controls (from chrome.storage.local under
//    `youtubeControls`, formatted by youtube-controls.js): Unhook-style
//    presentation, navigation, player and Shorts controls. Never weakens
//    or bypasses the session policy.
//
// Fail behavior for video pages:
// - When study mode, Watch Later-only, or blocked-channel rules apply,
//   any new video navigation immediately protects (pauses/mutes/covers)
//   the player until worker.js returns an explicit allow decision.
// - Unidentified videos fail closed after DETECTION_TIMEOUT_MS.
//
// Pure functions are exported on globalThis.__preelockYoutube for tests.
(function () {
  'use strict';
  var DETECTION_TIMEOUT_MS = 5000;
  var POLL_MS = 5000;
  var STYLE_ID = 'preelock-yt-style';
  var GUARD_ID = 'preelock-yt-guard';
  var SHORTS_COVER_ID = 'preelock-shorts-cover';
  var WL_KEY = 'preelock.watch-later.ids';
  var WL_URL = 'https://www.youtube.com/playlist?list=WL';
  var MAX_WL_IDS = 500;
  // Neutral cover shown while a video is being verified, so a blocked
  // video (or its thumbnail) never becomes visible first. It is
  // removed the moment the worker allows the video.
  var GUARD_CSS =
    '#' +
    GUARD_ID +
    '{position:fixed;inset:0;z-index:2147483000;display:flex;' +
    'align-items:center;justify-content:center;padding:2rem;' +
    'background:#101411;color:#e5e9e3;text-align:center;' +
    'font:500 15px/1.6 system-ui,-apple-system,sans-serif;letter-spacing:.01em}';

  var controlsApi =
    typeof globalThis !== 'undefined' && globalThis.__preelockYoutubeControls
      ? globalThis.__preelockYoutubeControls
      : null;

  function isYouTubeHost(hostname) {
    var host = String(hostname || '').toLowerCase();
    return host === 'youtube.com' || host.endsWith('.youtube.com');
  }
  function isMainYouTubeHost(hostname) {
    if (controlsApi && controlsApi.isMainYouTubeHost) {
      return controlsApi.isMainYouTubeHost(hostname);
    }
    var host = String(hostname || '').toLowerCase();
    return (
      host === 'youtube.com' ||
      host === 'www.youtube.com' ||
      host === 'm.youtube.com'
    );
  }
  function validHandle(body) {
    return (
      body.length >= 3 &&
      body.length <= 30 &&
      /^[a-z0-9](?:[a-z0-9._-]*[a-z0-9])?$/.test(body)
    );
  }
  function validUcId(id) {
    return /^UC[A-Za-z0-9_-]{10,62}$/.test(id);
  }
  function validLegacyName(name) {
    return (
      name.length >= 1 && name.length <= 100 && /^[a-z0-9._-]+$/.test(name)
    );
  }
  function channelKeyFromSegments(segments) {
    var first = segments[0];
    if (!first) return null;
    if (first.charAt(0) === '@') {
      var body = first.slice(1).toLowerCase();
      return validHandle(body) ? '@' + body : null;
    }
    if (first === 'channel') {
      var id = segments[1];
      return id && validUcId(id) ? id : null;
    }
    if (first === 'c' || first === 'user') {
      var name = segments[1] && segments[1].toLowerCase();
      return name && validLegacyName(name) ? first + ':' + name : null;
    }
    return null;
  }
  // The channel key for a YouTube URL pathname such as /@handle,
  // /@handle/videos, /channel/UC..., /c/name or /user/name, or null
  // for any other path.
  function parseChannelKeyFromPath(pathname) {
    var segments = String(pathname || '')
      .split('/')
      .filter(Boolean);
    return channelKeyFromSegments(segments);
  }
  // The channel key for one link href (relative or absolute), or null.
  function parseChannelKeyFromHref(href) {
    if (typeof href !== 'string' || !href) return null;
    var url;
    try {
      url = new URL(href, 'https://www.youtube.com/');
    } catch (e) {
      return null;
    }
    if (!isYouTubeHost(url.hostname)) return null;
    return parseChannelKeyFromPath(url.pathname);
  }
  // First href in the list that parses as a channel, or null.
  function extractChannelKey(hrefs) {
    for (var i = 0; i < (hrefs || []).length; i++) {
      var key = parseChannelKeyFromHref(hrefs[i]);
      if (key) return key;
    }
    return null;
  }
  // All unique canonical channel keys parsed from a list of hrefs or keys.
  function collectChannelKeys(inputs) {
    var out = [];
    var seen = {};
    for (var i = 0; i < (inputs || []).length; i++) {
      var raw = String(inputs[i] || '').trim();
      if (!raw) continue;
      var key = null;
      if (raw.charAt(0) === '@') {
        var body = raw.slice(1).toLowerCase();
        key = validHandle(body) ? '@' + body : null;
      } else if (validUcId(raw)) {
        key = raw;
      } else if (raw.indexOf('c:') === 0 || raw.indexOf('user:') === 0) {
        var parts = raw.split(':');
        var name = (parts[1] || '').toLowerCase();
        key = validLegacyName(name) ? parts[0] + ':' + name : null;
      } else {
        key = parseChannelKeyFromHref(raw);
      }
      if (key && !seen[key]) {
        seen[key] = true;
        out.push(key);
      }
    }
    return out;
  }
  // Lecture playback (problem B). Only the main YouTube site counts:
  // YouTube Music, Shorts, embeds and channel pages never verify a
  // lecture, so music or a short clip can never exempt the session from
  // its five-minute input requirement.
  var MIN_LECTURE_SECONDS = 120;
  function isLectureHost(hostname) {
    var host = String(hostname || '').toLowerCase();
    return (
      host === 'youtube.com' ||
      host === 'www.youtube.com' ||
      host === 'm.youtube.com'
    );
  }
  function isLecturePath(pathname) {
    var segments = String(pathname || '')
      .split('/')
      .filter(Boolean);
    if (segments[0] === 'watch') return true;
    return segments[0] === 'live' && segments.length >= 2;
  }
  // Measured playback only: the tab is a lecture page, its video is
  // playing and its playhead actually advanced since the previous
  // sample, no advertisement is showing, and the tab is visible. There
  // is no guess from a title, an open player or a loaded page.
  function lecturePlaying(state) {
    if (!state) return false;
    if (!state.lecturePage) return false;
    if (state.adShowing) return false;
    if (state.paused || state.ended) return false;
    if (!(state.currentTime > 0)) return false;
    if (!(state.duration >= MIN_LECTURE_SECONDS)) return false;
    if (state.visible === false) return false;
    return state.playheadAdvanced === true;
  }
  function isChannelLivePathname(pathname) {
    var segments = String(pathname || '')
      .split('/')
      .filter(Boolean);
    if (segments.length < 2) return false;
    if (segments[segments.length - 1].toLowerCase() !== 'live') return false;
    return Boolean(channelKeyFromSegments(segments));
  }
  // Video pages whose channel must be read from the DOM: watch, Shorts
  // and live pages. Embeds are excluded on purpose: an embed page
  // has no owner chip, so study mode would fail closed on allowed
  // channels' embeds; domain rules still apply to them.
  function isVideoPage(pathname) {
    var segments = String(pathname || '')
      .split('/')
      .filter(Boolean);
    if (segments[0] === 'watch') return true;
    if (
      (segments[0] === 'shorts' || segments[0] === 'live') &&
      segments.length >= 2
    )
      return true;
    return isChannelLivePathname(pathname);
  }
  function isShortsPath(pathname) {
    var path = String(pathname || '/').toLowerCase();
    return path === '/shorts' || path.startsWith('/shorts/');
  }
  // The homepage and the general feeds. With Watch Later as the only
  // permitted YouTube content these are sent to the playlist instead
  // of showing unrestricted browsing.
  function isHomeSurface(pathname) {
    var path = String(pathname || '/').toLowerCase();
    return path === '/' || path === '/feed' || path.startsWith('/feed/');
  }
  // The Watch Later *playlist* route (mirrors rules.js isWatchLaterUrl).
  function isWatchLaterUrl(urlString) {
    var url;
    try {
      url = new URL(urlString, 'https://www.youtube.com/');
    } catch (e) {
      return false;
    }
    if (!/^https?:$/.test(url.protocol)) return false;
    var host = url.hostname.toLowerCase();
    if (
      host !== 'youtube.com' &&
      host !== 'www.youtube.com' &&
      host !== 'm.youtube.com'
    )
      return false;
    if (url.pathname !== '/playlist' && url.pathname !== '/playlist/')
      return false;
    return url.searchParams.get('list') === 'WL';
  }
  // The Watch Later *video* route: a /watch page carrying list=WL.
  // This is a route, never a permission — membership is verified from
  // the playlist YouTube renders for the user (see
  // watchLaterMembership). Mirrors rules.js isWatchLaterVideoUrl.
  function isWatchLaterVideoUrl(urlString) {
    var url;
    try {
      url = new URL(urlString, 'https://www.youtube.com/');
    } catch (e) {
      return false;
    }
    if (!/^https?:$/.test(url.protocol)) return false;
    var host = url.hostname.toLowerCase();
    if (
      host !== 'youtube.com' &&
      host !== 'www.youtube.com' &&
      host !== 'm.youtube.com'
    )
      return false;
    if (url.pathname !== '/watch' && url.pathname !== '/watch/') return false;
    return (
      url.searchParams.get('list') === 'WL' &&
      Boolean(url.searchParams.get('v'))
    );
  }
  // The video id of one link href (/watch?v=...), or null.
  function videoIdFromHref(href) {
    if (typeof href !== 'string' || !href) return null;
    var url;
    try {
      url = new URL(href, 'https://www.youtube.com/');
    } catch (e) {
      return null;
    }
    if (!isYouTubeHost(url.hostname)) return null;
    if (url.pathname !== '/watch') return null;
    var id = url.searchParams.get('v');
    return id && /^[A-Za-z0-9_-]{5,20}$/.test(id) ? id : null;
  }
  function videoIdFromUrl(urlString) {
    return videoIdFromHref(urlString);
  }
  function videoIdFromVideoUrl(urlString) {
    if (controlsApi && controlsApi.videoIdFromVideoUrl) {
      return controlsApi.videoIdFromVideoUrl(urlString);
    }
    if (typeof urlString !== 'string' || !urlString) return null;
    var url;
    try {
      url = new URL(urlString, 'https://www.youtube.com/');
    } catch (e) {
      return null;
    }
    if (!isYouTubeHost(url.hostname)) return null;
    if (url.pathname === '/watch' || url.pathname === '/watch/') {
      var id = url.searchParams.get('v');
      return id && /^[A-Za-z0-9_-]{5,20}$/.test(id) ? id : null;
    }
    var parts = url.pathname.split('/').filter(Boolean);
    if ((parts[0] === 'shorts' || parts[0] === 'live') && parts[1]) {
      return /^[A-Za-z0-9_-]{5,20}$/.test(parts[1]) ? parts[1] : null;
    }
    return null;
  }
  // Unique video ids from a list of playlist link hrefs, in order.
  function videoIdsFromHrefs(hrefs) {
    var out = [];
    var seen = {};
    for (var i = 0; i < (hrefs || []).length; i++) {
      var id = videoIdFromHref(hrefs[i]);
      if (id && !seen[id]) {
        seen[id] = true;
        out.push(id);
      }
    }
    return out;
  }
  // True only when the video is one YouTube rendered as part of the
  // user's Watch Later playlist. `null` means "no evidence yet" (the
  // playlist panel may still be loading), never "allowed".
  function watchLaterMembership(videoId, ids) {
    if (!videoId) return null;
    return (ids || []).indexOf(videoId) === -1 ? null : true;
  }
  // Selectors hidden while a YouTube restriction is active. Only
  // surfaces that expose content the policy may not permit: the home
  // and general feeds, Shorts shelves and entries, and the
  // recommendation columns/end screens. The playlist panel that holds
  // the user's Watch Later items is deliberately untouched.
  function hiddenSelectors(policy, browsingClosed) {
    if (!restricted(policy)) return [];
    var selectors = [
      // Recommendation columns, shelves and end screens. The Watch
      // Later playlist panel is a sibling of the recommendations
      // column (verified against the live DOM), so it is not touched.
      'ytd-watch-next-secondary-results-renderer',
      'ytd-watch-flexy .ytp-endscreen-content',
      '.ytp-ce-element',
      // Shorts, in every shape YouTube currently renders them: shelves,
      // grid sections, feed shelves, and the guide/mini-guide entries.
      'ytd-reel-shelf-renderer',
      'ytd-rich-shelf-renderer[is-shorts]',
      'ytd-rich-section-renderer:has(ytd-rich-shelf-renderer[is-shorts])',
      'grid-shelf-view-model:has(a[href^="/shorts"])',
      'ytm-shorts-lockup-view-model',
      'ytd-guide-entry-renderer:has(a[href^="/shorts"])',
      'ytd-mini-guide-entry-renderer:has(a[href^="/shorts"])',
      'ytd-mini-guide-entry-renderer[aria-label="Shorts"]',
      'ytd-guide-entry-renderer:has(a[title="Shorts"])',
      '[data-preelock-blocked-card="true"]',
    ];
    if (browsingClosed) {
      selectors.push(
        'ytd-browse[page-subtype="home"] #contents',
        'ytd-browse[page-subtype="trending"] #contents',
        'ytd-browse[page-subtype="subscriptions"] #contents',
        'ytd-rich-grid-renderer',
        // Search is blocked route-by-route while browsing is closed,
        // so its box would only lead to the blocked page.
        'ytd-masthead #search-form',
        'ytd-masthead ytd-searchbox',
        'ytd-masthead #search-icon-legacy',
      );
    }
    return selectors;
  }
  // Any YouTube control is active for this policy (the script has work
  // to do even when youtube.com is otherwise reachable).
  function restricted(policy) {
    if (!policy) return false;
    return Boolean(
      policy.youtube_study_mode ||
      policy.youtube_block_entertainment ||
      (policy.youtube_blocked_channels || []).length ||
      (policy.youtube_allowed_channels || []).length ||
      policy.youtube_allow_watch_later,
    );
  }
  function isChannelRestrictedPolicy(policy, browsingClosed) {
    if (!policy) return false;
    return Boolean(
      policy.youtube_study_mode ||
      policy.youtube_block_entertainment ||
      (policy.youtube_blocked_channels || []).length ||
      (policy.youtube_allow_watch_later && browsingClosed),
    );
  }
  // Returns the Subscriptions URL to redirect to when Redirect to
  // Subscriptions is enabled and safe under the active policy, else null.
  function subscriptionsRedirectTarget(
    urlString,
    controls,
    policy,
    browsingClosed,
  ) {
    if (!controls || !controls.home || !controls.home.redirectToSubscriptions)
      return null;
    if (policy && policy.mode === 'offline') return null;
    if (policy && policy.youtube_allow_watch_later && browsingClosed)
      return null;
    if (policy && policy.youtube_study_mode) return null;
    var url;
    try {
      url = new URL(urlString || '');
    } catch (e) {
      return null;
    }
    if (!isMainYouTubeHost(url.hostname)) return null;
    if (url.pathname !== '/') return null;
    return url.origin + '/feed/subscriptions';
  }
  // The stylesheet for the current policy ('' when nothing applies).
  // Exposed for tests: the runtime applies exactly this text.
  function styleCss(policy, browsingClosed) {
    if (!restricted(policy)) return '';
    var css = GUARD_CSS;
    var selectors = hiddenSelectors(policy, browsingClosed);
    for (var i = 0; i < selectors.length; i++)
      css += '\n' + selectors[i] + '{display:none!important}';
    return css;
  }

  // Exposed for tests (and harmless in the isolated content-script
  // world, which the page itself cannot see).
  if (typeof globalThis !== 'undefined') {
    globalThis.__preelockYoutube = {
      DETECTION_TIMEOUT_MS: DETECTION_TIMEOUT_MS,
      MIN_LECTURE_SECONDS: MIN_LECTURE_SECONDS,
      isYouTubeHost: isYouTubeHost,
      isMainYouTubeHost: isMainYouTubeHost,
      parseChannelKeyFromPath: parseChannelKeyFromPath,
      parseChannelKeyFromHref: parseChannelKeyFromHref,
      extractChannelKey: extractChannelKey,
      collectChannelKeys: collectChannelKeys,
      isVideoPage: isVideoPage,
      isShortsPath: isShortsPath,
      isLectureHost: isLectureHost,
      isLecturePath: isLecturePath,
      lecturePlaying: lecturePlaying,
      isHomeSurface: isHomeSurface,
      isWatchLaterUrl: isWatchLaterUrl,
      isWatchLaterVideoUrl: isWatchLaterVideoUrl,
      videoIdFromHref: videoIdFromHref,
      videoIdFromUrl: videoIdFromUrl,
      videoIdFromVideoUrl: videoIdFromVideoUrl,
      videoIdsFromHrefs: videoIdsFromHrefs,
      watchLaterMembership: watchLaterMembership,
      hiddenSelectors: hiddenSelectors,
      restricted: restricted,
      isChannelRestrictedPolicy: isChannelRestrictedPolicy,
      subscriptionsRedirectTarget: subscriptionsRedirectTarget,
      styleCss: styleCss,
      PRODUCTIVE_EDUCATIONAL_PATTERN: PRODUCTIVE_EDUCATIONAL_PATTERN,
      STRONG_ENTERTAINMENT_PATTERN: STRONG_ENTERTAINMENT_PATTERN,
      isProductiveEducationalText: isProductiveEducationalText,
      isStrongEntertainmentText: isStrongEntertainmentText,
      isEntertainmentCardTitle: isEntertainmentCardTitle,
    };
  }

  // No extension context (tests, or a page that somehow loaded this
  // file directly): define the pure functions and stop.
  if (typeof chrome === 'undefined' || !chrome.runtime || !chrome.runtime.id)
    return;

  var lastUrl = '';
  var lastVideoId = '';
  var lastOwnerKey = null;
  var lastEvaluatedBridgeKey = '';
  var currentPreviewKey = '';
  var currentPreviewBlocked = false;
  var unverifiedSince = null;
  var pendingCheck = false;
  var evaluating = false;
  var reevaluateRequested = false;
  var retryTimer = null;
  var RETRY_WAIT_MS = 400;
  var lastRedirectAt = -10000;
  var lastSubRedirectAt = -10000;
  var lastAutoplayClickAt = -10000;
  var lastCancelClickAt = -10000;
  var guardActive = false;
  var guardMutedVideos = [];
  var guardOriginalMuted =
    typeof WeakMap !== 'undefined' ? new WeakMap() : null;
  var guardPausedVideos = typeof WeakSet !== 'undefined' ? new WeakSet() : null;
  var isSpaNavigation = false;
  var bridgeVideo = null;
  var oembedCache = {};
  var oembedPending = {};
  var currentPolicy = null;
  var currentBrowsingClosed = false;
  var currentControls = controlsApi
    ? controlsApi.normalizeYoutubeControls(null)
    : null;
  // The page/video/membership/policy combination that was last allowed,
  // so the 5 s poll never flashes the cover on a playing video.
  var allowedKey = null;
  var lastPolicyCss = null;
  var lastControlsCss = null;
  var lastRouteKind = '';

  function clearRetryEvaluate() {
    if (retryTimer !== null) {
      clearTimeout(retryTimer);
      retryTimer = null;
    }
  }

  function scheduleRetryEvaluate() {
    if (retryTimer !== null) return;
    retryTimer = setTimeout(function () {
      retryTimer = null;
      void evaluate();
    }, RETRY_WAIT_MS);
  }

  function sendMessage(message) {
    return new Promise(function (resolve) {
      var done = false;
      function finish(value) {
        if (!done) {
          done = true;
          resolve(value);
        }
      }
      // Timeout: if the worker doesn't respond in 3s, don't hang evaluate().
      setTimeout(function () {
        finish(null);
      }, 3000);
      try {
        chrome.runtime.sendMessage(message, function (response) {
          if (chrome.runtime.lastError) finish(null);
          else finish(response || null);
        });
      } catch (e) {
        finish(null);
      }
    });
  }

  function isInlinePreviewVideo(video) {
    try {
      return Boolean(
        video && video.closest && video.closest('#inline-preview-player'),
      );
    } catch (e) {
      return false;
    }
  }

  function allVideos() {
    try {
      return document.querySelectorAll('video') || [];
    } catch (e) {
      return [];
    }
  }

  function stopVideo() {
    var videos = allVideos();
    if (!videos.length) {
      try {
        var single = document.querySelector('video');
        if (single) {
          if (single.pause) single.pause();
          single.muted = true;
          single.currentTime = 0;
        }
      } catch (e) {}
      return;
    }
    for (var i = 0; i < videos.length; i++) {
      try {
        if (videos[i]) {
          if (videos[i].pause) videos[i].pause();
          videos[i].muted = true;
          videos[i].currentTime = 0;
        }
      } catch (e) {}
    }
  }

  function protectPlayback(video) {
    if (!video) return;
    try {
      if (video.paused === false && guardPausedVideos) {
        guardPausedVideos.add(video);
      }
      if (video.pause) video.pause();
    } catch (e) {}
    try {
      if (typeof video.muted === 'boolean') {
        if (guardOriginalMuted && !guardOriginalMuted.has(video)) {
          guardOriginalMuted.set(video, video.muted);
        }
        if (guardMutedVideos.indexOf(video) === -1)
          guardMutedVideos.push(video);
        if (video.muted === false) {
          video.muted = true;
        }
      }
    } catch (e) {}
  }

  function collectOwnerHrefs(currentVideoId) {
    var hrefs = [];
    var seen = {};
    function add(href) {
      if (href && !seen[href]) {
        seen[href] = true;
        hrefs.push(href);
      }
    }
    // Check bridge identity verified for the current video first.
    if (
      bridgeVideo &&
      (!currentVideoId ||
        !bridgeVideo.videoId ||
        bridgeVideo.videoId === currentVideoId ||
        bridgeVideo.playerSwapped)
    ) {
      if (bridgeVideo.channelId && validUcId(bridgeVideo.channelId)) {
        add('/channel/' + bridgeVideo.channelId);
      }
      if (bridgeVideo.ownerUrl) {
        add(bridgeVideo.ownerUrl);
      }
    }
    var isShorts = false;
    try {
      isShorts = isShortsPath(location.pathname);
    } catch (e) {}
    // Avoid reading stale watch DOM before ytd-watch-flexy updates its video-id.
    var domStale = false;
    if (!isShorts) {
      try {
        var flexy =
          document.querySelector('ytd-watch-flexy:not([hidden])') ||
          document.querySelector('ytd-watch-flexy');
        if (flexy) {
          var isHiddenFlexy =
            typeof flexy.hasAttribute === 'function' &&
            flexy.hasAttribute('hidden');
          var flexyId =
            typeof flexy.getAttribute === 'function'
              ? flexy.getAttribute('video-id')
              : undefined;
          if (
            isHiddenFlexy ||
            (currentVideoId &&
              ((flexyId && flexyId !== currentVideoId) ||
                (!flexyId &&
                  typeof flexy.hasAttribute === 'function' &&
                  (isSpaNavigation ||
                    (bridgeVideo && bridgeVideo.playerSwapped)))))
          ) {
            domStale = true;
          }
        }
      } catch (e) {}
    }
    if (!domStale) {
      var nodes;
      try {
        var selector = isShorts
          ? 'ytd-reel-video-renderer[is-active] ytd-channel-name a[href], ' +
            'ytd-reel-video-renderer[is-active] #channel-name a[href], ' +
            'ytd-reel-video-renderer[is-active] ytd-reel-player-header-renderer a[href], ' +
            'ytd-reel-video-renderer[is-active] a[href^="/@"], ' +
            'ytd-reel-video-renderer[is-active] a[href^="/channel/"], ' +
            'ytd-reel-video-renderer[is-active] a[href^="/c/"], ' +
            'ytd-reel-video-renderer[is-active] a[href^="/user/"]'
          : 'ytd-watch-flexy:not([hidden]) #above-the-fold #owner a[href], ' +
            'ytd-watch-flexy:not([hidden]) ytd-watch-metadata #owner a[href], ' +
            'ytd-watch-flexy:not([hidden]) #upload-info ytd-channel-name a[href], ' +
            'ytd-watch-flexy:not([hidden]) ytd-video-owner-renderer a[href], ' +
            'ytm-slim-owner-renderer a[href]';
        nodes = document.querySelectorAll(selector);
      } catch (e) {
        nodes = [];
      }
      for (var i = 0; i < nodes.length; i++) {
        var href = nodes[i].getAttribute
          ? nodes[i].getAttribute('href')
          : nodes[i].href;
        add(href);
      }
    }
    // Head meta tags only reflect the initial load; YouTube leaves them
    // unchanged during SPA transitions or internal player swaps, so never
    // trust them after SPA nav or player swap.
    if (!isSpaNavigation && !(bridgeVideo && bridgeVideo.playerSwapped)) {
      try {
        var meta = document.querySelector('meta[itemprop="channelId"]');
        var cid = meta && meta.getAttribute && meta.getAttribute('content');
        if (cid) add('/channel/' + cid);
        var link = document.querySelector('link[itemprop="url"]');
        var lhref = link && link.getAttribute && link.getAttribute('href');
        if (lhref) add(lhref);
      } catch (e) {}
    }
    return hrefs;
  }

  // Links YouTube renders inside the user's Watch Later playlist: its
  // playlist page, or a playlist panel whose items carry list=WL.
  // Anything else is not membership — a recommendation, a search result
  // or a different playlist panel (a Mix, a course playlist) is never
  // evidence. On the Watch Later playlist page the page itself proves
  // which playlist the items belong to.
  function collectPlaylistHrefs() {
    var onWatchLaterPage = false;
    try {
      onWatchLaterPage = isWatchLaterUrl(location.href);
    } catch (e) {
      onWatchLaterPage = false;
    }
    var hrefs = [];
    var nodes;
    try {
      nodes = document.querySelectorAll(
        'ytd-playlist-video-list-renderer a[href*="watch?v="], ' +
          'ytd-playlist-panel-video-renderer a[href*="watch?v="], ' +
          'ytd-playlist-panel-renderer a[href*="watch?v="]',
      );
    } catch (e) {
      nodes = [];
    }
    for (var i = 0; i < nodes.length; i++) {
      var href = nodes[i].getAttribute
        ? nodes[i].getAttribute('href')
        : nodes[i].href;
      if (!href) continue;
      if (isWatchLaterVideoUrl(href)) hrefs.push(href);
      else if (onWatchLaterPage && videoIdFromHref(href)) hrefs.push(href);
    }
    return hrefs;
  }
  function loadWlIds() {
    try {
      var raw = sessionStorage.getItem(WL_KEY);
      var parsed = raw ? JSON.parse(raw) : [];
      if (!Array.isArray(parsed)) return [];
      return parsed.filter(function (id) {
        return typeof id === 'string';
      });
    } catch (e) {
      return [];
    }
  }
  function saveWlIds(ids) {
    try {
      sessionStorage.setItem(WL_KEY, JSON.stringify(ids.slice(-MAX_WL_IDS)));
    } catch (e) {}
  }
  function forgetWlIds() {
    try {
      sessionStorage.removeItem(WL_KEY);
    } catch (e) {}
  }
  function rememberWlIds() {
    var found = videoIdsFromHrefs(collectPlaylistHrefs());
    var known = loadWlIds();
    if (!found.length) return known;
    var all = known.slice();
    for (var i = 0; i < found.length; i++)
      if (all.indexOf(found[i]) === -1) all.push(found[i]);
    saveWlIds(all);
    return all;
  }

  function upsertStyle(id, css, cacheRef) {
    var style = null;
    try {
      style = document.getElementById(id);
    } catch (e) {}
    if (!css) {
      if (style && style.remove) style.remove();
      return '';
    }
    if (style && cacheRef === css) return css;
    if (!style) {
      try {
        style = document.createElement('style');
        style.id = id;
        var root = document.head || document.documentElement;
        if (!root) return '';
        root.appendChild(style);
      } catch (e) {
        return '';
      }
    }
    style.textContent = css;
    return css;
  }

  function applyStyles(policy, browsingClosed) {
    var css = styleCss(policy, browsingClosed);
    lastPolicyCss = upsertStyle(STYLE_ID, css, lastPolicyCss);
  }

  function applyControlsStyles() {
    if (!controlsApi || !currentControls) return;
    try {
      var route = controlsApi.youtubeRouteKind(location.href);
      if (
        route !== lastRouteKind &&
        document.documentElement &&
        document.documentElement.setAttribute
      ) {
        document.documentElement.setAttribute('data-preelock-route', route);
        lastRouteKind = route;
      }
    } catch (e) {}
    var css = controlsApi.controlsCss(currentControls);
    lastControlsCss = upsertStyle(
      controlsApi.CONTROLS_STYLE_ID,
      css,
      lastControlsCss,
    );
  }

  function showGuard() {
    guardActive = true;
    try {
      if (!document.getElementById(GUARD_ID)) {
        var node = document.createElement('div');
        node.id = GUARD_ID;
        node.setAttribute('role', 'status');
        node.textContent = 'Preelock \u00b7 Checking this video\u2026';
        var parent = document.body || document.documentElement;
        if (parent) parent.appendChild(node);
      }
      var videos = allVideos();
      if (videos.length) {
        for (var i = 0; i < videos.length; i++) {
          if (!isInlinePreviewVideo(videos[i])) protectPlayback(videos[i]);
        }
      } else {
        var single = document.querySelector('video');
        if (single && !isInlinePreviewVideo(single)) protectPlayback(single);
      }
    } catch (e) {}
  }

  function hideGuard() {
    guardActive = false;
    try {
      var node = document.getElementById(GUARD_ID);
      if (node && node.remove) node.remove();
    } catch (e) {}
    try {
      for (var i = 0; i < guardMutedVideos.length; i++) {
        var mv = guardMutedVideos[i];
        if (!mv || isInlinePreviewVideo(mv)) continue;
        if (guardOriginalMuted && guardOriginalMuted.has(mv)) {
          mv.muted = guardOriginalMuted.get(mv);
          guardOriginalMuted.delete(mv);
        } else {
          mv.muted = false;
        }
      }
    } catch (e) {}
    guardMutedVideos = [];
    if (guardPausedVideos) {
      var videos = allVideos();
      for (var j = 0; j < videos.length; j++) {
        var v = videos[j];
        if (guardPausedVideos.has(v)) {
          guardPausedVideos.delete(v);
          try {
            if (v.paused && !v.ended && typeof v.play === 'function') {
              var p = v.play();
              if (p && typeof p.catch === 'function') p.catch(function () {});
            }
          } catch (e) {}
        }
      }
    }
  }

  function showShortsCover() {
    stopVideo();
    var videos = allVideos();
    for (var i = 0; i < videos.length; i++) {
      try {
        if (videos[i]) videos[i].muted = true;
      } catch (e) {}
    }
    try {
      if (document.getElementById(SHORTS_COVER_ID)) return;
      var cover = document.createElement('div');
      cover.id = SHORTS_COVER_ID;
      cover.setAttribute('role', 'region');
      cover.setAttribute('aria-label', 'Preelock Shorts restriction');

      var card = document.createElement('div');
      card.className = 'preelock-shorts-card';
      var heading = document.createElement('h2');
      heading.textContent = 'YouTube Shorts hidden';
      var msg = document.createElement('p');
      msg.textContent =
        'Shorts are hidden by your Preelock YouTube Controls settings.';
      var btn = document.createElement('button');
      btn.type = 'button';
      btn.textContent = 'Open Subscriptions';
      btn.addEventListener('click', function () {
        try {
          location.replace(location.origin + '/feed/subscriptions');
        } catch (e) {}
      });
      card.appendChild(heading);
      card.appendChild(msg);
      card.appendChild(btn);
      cover.appendChild(card);
      var parent = document.body || document.documentElement;
      if (parent) parent.appendChild(cover);
    } catch (e) {}
  }

  function hideShortsCover() {
    try {
      var cover = document.getElementById(SHORTS_COVER_ID);
      if (cover && cover.remove) cover.remove();
    } catch (e) {}
  }

  function clearUi() {
    hideGuard();
    applyStyles(null, false);
    forgetWlIds();
    clearAttrMarks('data-preelock-blocked-card');
  }

  function redirectToWatchLater() {
    var now = Date.now();
    // One redirect per navigation burst: no loop even if the playlist
    // itself somehow fails to load.
    if (now - lastRedirectAt < 2000) return;
    lastRedirectAt = now;
    try {
      location.replace(WL_URL);
    } catch (e) {}
  }

  function redirectToSubscriptions(targetUrl) {
    var now = Date.now();
    if (now - lastSubRedirectAt < 1500) return;
    lastSubRedirectAt = now;
    try {
      location.replace(targetUrl);
    } catch (e) {}
  }

  function requestBridgeRead() {
    try {
      if (typeof window.postMessage === 'function') {
        window.postMessage(
          { source: 'preelock-content', type: 'read-video' },
          location.origin || '*',
        );
      }
    } catch (e) {}
  }

  function syncBridgeAutoplay(disabled) {
    try {
      if (typeof window.postMessage === 'function') {
        window.postMessage(
          {
            source: 'preelock-content',
            type: 'set-autoplay',
            disabled: Boolean(disabled),
          },
          location.origin || '*',
        );
      }
    } catch (e) {}
  }

  function enforceDisableAutoplay() {
    if (!currentControls || !currentControls.player.disableAutoplay) return;
    syncBridgeAutoplay(true);
    try {
      var btn = document.querySelector('.ytp-autonav-toggle-button');
      if (btn && btn.getAttribute('aria-checked') === 'true') {
        var now = Date.now();
        if (now - lastAutoplayClickAt >= 700) {
          lastAutoplayClickAt = now;
          if (typeof btn.click === 'function') btn.click();
        }
      }
      var cancelBtn = document.querySelector(
        '.ytp-autonav-endscreen-upnext-cancel-button',
      );
      if (cancelBtn && typeof cancelBtn.click === 'function') {
        var container = document.querySelector(
          '.ytp-autonav-endscreen-upnext-container',
        );
        if (
          (!container || container.style.display !== 'none') &&
          Date.now() - lastCancelClickAt >= 700
        ) {
          lastCancelClickAt = Date.now();
          cancelBtn.click();
        }
      }
    } catch (e) {}
  }

  function clearAttrMarks(attr, valueFilter) {
    try {
      var selector = valueFilter
        ? '[' + attr + '="' + valueFilter + '"]'
        : '[' + attr + ']';
      var marked = document.querySelectorAll(selector);
      for (var i = 0; i < marked.length; i++) {
        marked[i].removeAttribute(attr);
      }
    } catch (e) {}
  }

  function markAttr(el, attr, val) {
    if (!el || !el.getAttribute || !el.setAttribute) return;
    if (el.getAttribute(attr) !== val) el.setAttribute(attr, val);
  }

  function applyDynamicDomControls() {
    // 1. Search page Shorts chip
    if (currentControls && currentControls.recommendations.hideShorts) {
      try {
        if (location.pathname === '/results') {
          var chips = document.querySelectorAll(
            'yt-chip-cloud-chip-renderer, chip-shape',
          );
          for (var i = 0; i < chips.length; i++) {
            var text = String(chips[i].textContent || '').trim();
            if (text === 'Shorts') {
              markAttr(chips[i], 'data-preelock-hidden', 'shorts');
            }
          }
        }
      } catch (e) {}
    } else {
      clearAttrMarks('data-preelock-hidden', 'shorts');
    }

    // 2. Explore and Subscriptions sections in left sidebar guide
    var hideExplore = Boolean(
      currentControls && currentControls.navigation.hideExploreAndTrending,
    );
    var hideSubs = Boolean(
      currentControls && currentControls.navigation.hideSubscriptions,
    );
    if (hideExplore || hideSubs) {
      try {
        var sections = document.querySelectorAll('ytd-guide-section-renderer');
        for (var j = 0; j < sections.length; j++) {
          var sec = sections[j];
          var titleEl =
            sec.querySelector && sec.querySelector('#guide-section-title');
          var title = titleEl ? String(titleEl.textContent || '').trim() : '';
          var titleLower = title.toLowerCase();
          if (hideExplore && titleLower === 'explore') {
            markAttr(sec, 'data-preelock-hidden', 'explore');
            continue;
          }
          if (hideSubs) {
            var isProtectedSection = Boolean(
              sec.querySelector &&
              sec.querySelector(
                'a[href="/"], a[href^="/shorts"], a[href="/playlist?list=WL"], ' +
                  'a[href="/playlist?list=LL"], a[href="/feed/you"], a[href="/feed/history"], ' +
                  'a[href="/feed/library"], a[href="/feed/playlists"], a[href="/reporthistory"], ' +
                  'a[href="/premium"]',
              ),
            );
            var isExploreOrMore =
              titleLower === 'explore' ||
              titleLower === 'more from youtube' ||
              Boolean(
                sec.querySelector &&
                sec.querySelector(
                  'a[href="/feed/trending"], a[href="/gaming"], a[href="/podcasts"], ' +
                    'a[href="/playables"], a[href^="/feed/storefront"], ' +
                    'a[href="/channel/UC-9-kyTW8ZkZNDHQJ6FgpwQ"], ' +
                    'a[href="/channel/UC4R8DWoMoI7CAwX8_LjQHig"]',
                ),
              );
            var isSubsSection =
              !isProtectedSection &&
              !isExploreOrMore &&
              (titleLower === 'subscriptions' ||
                Boolean(
                  sec.querySelector &&
                  sec.querySelector(
                    'a[href="/feed/subscriptions"], a[title="Subscriptions"], ' +
                      'a[href="/feed/channels"], ytd-guide-entry-renderer[line-end-style="dot"], ' +
                      'a[href^="/@"], a[href^="/c/"], a[href^="/user/"], a[href^="/channel/"]',
                  ),
                ));
            if (isSubsSection) {
              markAttr(sec, 'data-preelock-hidden', 'subscriptions');
            } else if (
              sec.getAttribute &&
              sec.getAttribute('data-preelock-hidden') === 'subscriptions'
            ) {
              sec.removeAttribute('data-preelock-hidden');
            }
          }
        }
      } catch (e) {}
    }
    if (!hideExplore) clearAttrMarks('data-preelock-hidden', 'explore');
    if (!hideSubs) clearAttrMarks('data-preelock-hidden', 'subscriptions');

    // 3. Mixes badge detection for newer lockups
    if (currentControls && currentControls.recommendations.hideMixes) {
      try {
        var cards = document.querySelectorAll(
          'yt-lockup-view-model, ytd-rich-item-renderer, ytd-compact-video-renderer',
        );
        for (var k = 0; k < cards.length; k++) {
          var badge =
            cards[k].querySelector &&
            cards[k].querySelector('badge-shape, .badge-style-type-simple');
          var badgeText = badge
            ? String(badge.textContent || '')
                .trim()
                .toLowerCase()
            : '';
          if (badgeText === 'mix') {
            markAttr(cards[k], 'data-preelock-hidden', 'mix');
          }
        }
      } catch (e) {}
    } else {
      clearAttrMarks('data-preelock-hidden', 'mix');
    }

    // 4. Blocked-channel & entertainment cards across Search, Subscriptions, Home, Recommendations & Playlists
    var blockedChannels =
      (currentPolicy && currentPolicy.youtube_blocked_channels) || [];
    var blockEntertainment = Boolean(
      currentPolicy && currentPolicy.youtube_block_entertainment,
    );
    if (blockedChannels.length || blockEntertainment) {
      try {
        var cardNodes = document.querySelectorAll(
          'ytd-video-renderer, ytd-rich-item-renderer, ytd-grid-video-renderer, ' +
            'ytd-compact-video-renderer, ytd-playlist-video-renderer, ' +
            'ytd-playlist-panel-video-renderer, yt-lockup-view-model, ytd-channel-renderer, ' +
            'ytd-reel-item-renderer',
        );
        for (var m = 0; m < cardNodes.length; m++) {
          var card = cardNodes[m];
          var hit = false;
          if (blockedChannels.length) {
            var links =
              card.querySelectorAll &&
              card.querySelectorAll(
                'a[href^="/@"], a[href^="/channel/"], a[href^="/c/"], a[href^="/user/"]',
              );
            for (var n = 0; links && n < links.length; n++) {
              var href = links[n].getAttribute
                ? links[n].getAttribute('href')
                : links[n].href;
              var key = parseChannelKeyFromHref(href);
              if (key && blockedChannels.indexOf(key) !== -1) {
                hit = true;
                break;
              }
            }
          }
          if (!hit && blockEntertainment) {
            var shortLink =
              card.querySelector &&
              card.querySelector('a[href^="/shorts/"], a[href*="/shorts/"]');
            if (shortLink) {
              hit = true;
            } else {
              var titleEl =
                card.querySelector &&
                card.querySelector(
                  '#video-title, h3 a, .yt-lockup-metadata-view-model__title, span#video-title',
                );
              var titleText = titleEl ? String(titleEl.textContent || '') : '';
              if (titleText && isEntertainmentCardTitle(titleText)) hit = true;
            }
          }
          if (hit) markAttr(card, 'data-preelock-blocked-card', 'true');
          else if (
            card.hasAttribute &&
            card.hasAttribute('data-preelock-blocked-card')
          ) {
            card.removeAttribute('data-preelock-blocked-card');
          }
        }
      } catch (e) {}
    } else {
      clearAttrMarks('data-preelock-blocked-card');
    }

    enforceDisableAutoplay();
  }

  var dynamicTimer = null;
  function scheduleDynamicControls() {
    if (dynamicTimer) return;
    dynamicTimer = setTimeout(function () {
      dynamicTimer = null;
      applyDynamicDomControls();
    }, 80);
  }

  async function channelFromOEmbed(href, videoId) {
    var cacheKey = videoId || href;
    if (Object.prototype.hasOwnProperty.call(oembedCache, cacheKey)) {
      return oembedCache[cacheKey];
    }
    if (oembedPending[cacheKey]) return oembedPending[cacheKey];
    var targetUrl = videoId
      ? 'https://www.youtube.com/watch?v=' + encodeURIComponent(videoId)
      : href;
    oembedPending[cacheKey] = (async function () {
      try {
        var resp = await fetch(
          'https://www.youtube.com/oembed?url=' +
            encodeURIComponent(targetUrl) +
            '&format=json',
        );
        if (!resp || !resp.ok) {
          oembedCache[cacheKey] = null;
          return null;
        }
        var data = await resp.json();
        var key =
          data && data.author_url
            ? parseChannelKeyFromHref(data.author_url)
            : null;
        oembedCache[cacheKey] = key;
        return key;
      } catch (e) {
        return null;
      } finally {
        delete oembedPending[cacheKey];
      }
    })();
    return oembedPending[cacheKey];
  }

  var PRODUCTIVE_EDUCATIONAL_PATTERN =
    /(?:\b(?:procrastinat(?:ion|e|ing)?|discipline|self[\s-]?discipline|deep[\s-]?work|focus(?:ed|ing)?|habit(?:s)?|atomic\s+habits|routine(?:s)?|dopamine(?:\s+detox)?|hard\s+(?:things|work)|willpower|mental\s+toughness|monk\s+mode|mindset|stoic(?:ism)?|self[\s-]?improvement|personal\s+development|productivity|productive|time[\s-]?management|pomodoro|digital\s+detox|life\s+without\s+social\s+media|quit(?:ting)?\s+(?:social\s+media|phone|sugar|bad\s+habits|porn|gaming)|social\s+media\s+addiction|phone\s+addiction|stop\s+playing\s+small|burnout|motivation(?:al)?|explain(?:ed|ing|s)?|explanation(?:s)?|how\s+to(?!\s+(?:prank|roast|troll|meme|cheat))\b|why\s+(?:we|you|i|it|is|do|people|they)\b|here['’]?s\s+why|tutorial(?:s)?|guide(?:\s+to)?|beginner['’]?s\s+guide|ultimate\s+guide|course(?:s)?|crash\s+course|masterclass|lecture(?:s)?|lesson(?:s)?|learn(?:ing)?|study(?:ing)?|study\s+skills|study\s+with\s+me|breakdown|analysis|analyzing|deep\s+dive|overview|summary|book\s+summary|case\s+study|documentary|documentaries|video\s+essay(?:s)?|neuroscience|psychology|philosophy|science(?:s)?|scientific|physics|chemistry|biology|astronomy|mathematics|math|algebra|calculus|geometry|statistics|history(?: of)?|historical|geopolitics|geopolitical|economics|finance|investing|stock\s+market|business|career|interview\s+prep|coding|programming|developer(?:s)?|software\s+engine(?:er|ering)|computer\s+science|algorithm(?:s)?|data\s+structures?|system\s+design|web\s+dev(?:elopment)?|machine\s+learning|deep\s+learning|artificial\s+intelligence|\bAI\b|neural\s+network(?:s)?|python|javascript|typescript|rust|golang|c\+\+|java\b|react(?:js|\.js)?|node(?:\.js)?|sql|linux|docker|kubernetes|ted\s+talk(?:s)?|\btedx\b|keynote|discussion|podcast|interview(?:s)?)\b)/i;

  var STRONG_ENTERTAINMENT_PATTERN =
    /(?:\b(?:stand[\s-]?up\s+comed(?:y|ian)|stand[\s-]?up\s+special|stand[\s-]?up\b|comedian(?:s)?|comedy\s+(?:show|scenes?|special|club|night|circus|sketch|skit|clash|king|video)|comedy\b|comedic\b|sitcom(?:s)?|roast(?:s|ed|ing)?\s+show|comedy\s+roast|roasting\s+(?:the\s+)?.*(?:tiktokers|youtubers|streamers|celebrities)|roast\s+of\b|sketch\s+comedy|comedy\s+skit(?:s)?|parod(?:y|ies)|spoof(?:s)?|prank(?:s|ed|ing|ster)?|social\s+experiment|try\s+not\s+to\s+laugh|fail\s+compilation(?:s)?|bloopers?|cringe\s+compilation(?:s)?|memes?|shitpost(?:s|ing)?|dank\s+memes?|funny\s+(?:videos?|clips?|moments?|fails?|animals?|pets?|cats?|dogs?|cats\s+and\s+dogs)|(?:cute|cutest)\s+(?:animals?|pets?|cats?|dogs?|puppies|kittens)|funniest\s+(?:videos?|clips?|moments?|fails?|animals?)|hilarious\s+(?:clips?|moments?|fails?|comedy)|cat\s+videos?|dog\s+videos?|gameplay(?:\s+walkthrough)?|playthrough|let['’]?s\s+play|gaming\s+montage|funny\s+gaming\s+moments|speedrun|mukbang|eating\s+show|food\s+challenge|last\s+to\s+leave|24\s+hours\s+(?:in|at|with)\s+challenge|official\s+(?:movie\s+)?trailer|teaser\s+trailer|movie\s+trailer|film\s+trailer|official\s+teaser|teaser\b|trailer\b|movie\s+clips?|official\s+music\s+video|official\s+lyric\s+video|music\s+video|celebrity\s+gossip|paparazzi|drama\s+alert|tea\s+spill|countdown\s+to\s+.*\b(?:avengers|marvel|disney\+|star\s+wars|dc|batman|superman)\b|daily\s+vlog(?:s)?|family\s+vlog(?:s)?|travel\s+vlog(?:s)?|lifestyle\s+vlog(?:s)?|vacation\s+vlog(?:s)?|\bgrwm\b|get\s+ready\s+with\s+me|day\s+in\s+my\s+life\s+vlog)\b|#(?:comedy|standup|prank|pranks|dailyvlog|minivlog|meme|memes|pets|animals|trynottolaugh|mukbang|gaming)\b)/i;

  function isProductiveEducationalText(text) {
    return (
      typeof text === 'string' && PRODUCTIVE_EDUCATIONAL_PATTERN.test(text)
    );
  }

  function isStrongEntertainmentText(text) {
    return (
      typeof text === 'string' && STRONG_ENTERTAINMENT_PATTERN.test(text)
    );
  }

  function isEntertainmentCardTitle(text) {
    if (typeof text !== 'string' || !text.trim()) return false;
    var isProd = isProductiveEducationalText(text);
    var isEnt = isStrongEntertainmentText(text);
    if (isProd && !isEnt) return false;
    if (isEnt) {
      if (!isProd) return true;
      return /\b(?:stand[\s-]?up|comed(?:y|ian)|prank|roast|meme|mukbang|let['’]?s\s+play|gameplay|try\s+not\s+to\s+laugh|parod(?:y|ies)|vlog)\b/i.test(
        text,
      );
    }
    return false;
  }

  function extractCleanDocumentTitle() {
    try {
      if (!document.title) return '';
      var raw = String(document.title).trim();
      var cleaned = raw.replace(/^\(\d+[\d+]*\)\s*/, '');
      cleaned = cleaned.replace(/\s*[-–—|]\s*YouTube$/i, '').trim();
      if (!cleaned || cleaned.toLowerCase() === 'youtube') return '';
      return cleaned;
    } catch (e) {
      return '';
    }
  }

  function extractDomVideoTitle() {
    var selectors = [
      'ytd-watch-metadata #title h1 yt-formatted-string',
      'ytd-watch-metadata #title h1',
      'ytd-watch-metadata #title yt-formatted-string',
      'ytd-watch-metadata h1 yt-formatted-string',
      'ytd-watch-metadata h1',
      '#above-the-fold #title h1 yt-formatted-string',
      '#above-the-fold #title h1',
      '#above-the-fold #title yt-formatted-string',
      '#above-the-fold #title',
      '#title.ytd-watch-metadata',
      '#title h1 yt-formatted-string',
      '#title h1',
      'h1.ytd-watch-metadata',
      'h1.title.style-scope.ytd-video-primary-info-renderer',
      'h1.title',
      'ytd-reel-video-renderer[is-active] .title',
      'ytd-reel-video-renderer[is-active] #reel-player-header-title',
      'ytd-reel-player-header-renderer h2',
    ];
    for (var i = 0; i < selectors.length; i++) {
      try {
        var el = document.querySelector(selectors[i]);
        if (el) {
          var txt = String(el.textContent || '').trim();
          if (txt) return txt;
        }
      } catch (e) {}
    }
    var metaSelectors = [
      'meta[name="title"]',
      'meta[property="og:title"]',
      'meta[property="twitter:title"]',
      'meta[name="twitter:title"]',
      'meta[itemprop="name"]',
    ];
    for (var j = 0; j < metaSelectors.length; j++) {
      try {
        var m = document.querySelector(metaSelectors[j]);
        if (m && m.getAttribute) {
          var c = String(m.getAttribute('content') || '').trim();
          if (c) {
            c = c.replace(/\s*[-–—|]\s*YouTube$/i, '').trim();
            if (c && c.toLowerCase() !== 'youtube') return c;
          }
        }
      } catch (e) {}
    }
    return '';
  }

  function extractDomVideoCategory() {
    var metaSelectors = [
      'meta[itemprop="genre"]',
      'meta[property="video:category"]',
      'meta[name="video:category"]',
    ];
    for (var i = 0; i < metaSelectors.length; i++) {
      try {
        var m = document.querySelector(metaSelectors[i]);
        if (m && m.getAttribute) {
          var c = String(m.getAttribute('content') || '').trim();
          if (c) return c;
        }
      } catch (e) {}
    }
    try {
      var ldNodes = document.querySelectorAll(
        'script[type="application/ld+json"]',
      );
      for (var j = 0; j < ldNodes.length; j++) {
        var rawJson = ldNodes[j].textContent || '';
        if (rawJson && rawJson.indexOf('"genre"') !== -1) {
          var parsed = JSON.parse(rawJson);
          if (
            parsed &&
            typeof parsed.genre === 'string' &&
            parsed.genre.trim()
          ) {
            return parsed.genre.trim();
          }
        }
      }
    } catch (e) {}
    return '';
  }

  function extractDomAuthor() {
    var selectors = [
      'ytd-watch-metadata #channel-name yt-formatted-string a',
      'ytd-watch-metadata #channel-name a',
      'ytd-watch-metadata #channel-name',
      '#above-the-fold #channel-name yt-formatted-string a',
      '#above-the-fold #channel-name a',
      '#above-the-fold #channel-name',
      '#owner #channel-name a',
      '#owner #channel-name',
      'ytd-reel-video-renderer[is-active] #channel-name a',
      'ytd-reel-player-header-renderer #channel-name a',
    ];
    for (var i = 0; i < selectors.length; i++) {
      try {
        var el = document.querySelector(selectors[i]);
        if (el) {
          var txt = String(el.textContent || '').trim();
          if (txt) return txt;
        }
      } catch (e) {}
    }
    try {
      var authorMeta = document.querySelector(
        'meta[itemprop="author"], link[itemprop="name"]',
      );
      if (authorMeta && authorMeta.getAttribute) {
        var c = String(authorMeta.getAttribute('content') || '').trim();
        if (c) return c;
      }
    } catch (e) {}
    return '';
  }

  function collectVideoMeta(activeBridge) {
    var category =
      activeBridge && typeof activeBridge.category === 'string'
        ? activeBridge.category.trim()
        : '';
    var title =
      activeBridge && typeof activeBridge.title === 'string'
        ? activeBridge.title.trim()
        : '';
    var author =
      activeBridge && typeof activeBridge.author === 'string'
        ? activeBridge.author.trim()
        : '';
    var keywords =
      activeBridge && Array.isArray(activeBridge.keywords)
        ? activeBridge.keywords.slice()
        : [];
    var hashtags = [];

    if (!category) {
      category = extractDomVideoCategory();
    }
    if (!title) {
      title = extractDomVideoTitle();
    }
    if (!title) {
      title = extractCleanDocumentTitle();
    }
    if (!author) {
      author = extractDomAuthor();
    }

    try {
      var hashNodes = document.querySelectorAll(
        'ytd-watch-metadata a[href*="/hashtag/"], #above-the-fold a[href*="/hashtag/"], ytd-reel-player-header-renderer a[href*="/hashtag/"]',
      );
      for (var i = 0; i < hashNodes.length && hashtags.length < 15; i++) {
        var txt = String(hashNodes[i].textContent || '').trim();
        if (txt && hashtags.indexOf(txt) === -1) hashtags.push(txt);
      }
    } catch (e) {}

    if (!keywords.length) {
      try {
        var kwNode = document.querySelector('meta[name="keywords"]');
        if (kwNode && kwNode.getAttribute) {
          var kwStr = String(kwNode.getAttribute('content') || '').trim();
          if (kwStr) {
            var spl = kwStr.split(',');
            for (var k = 0; k < spl.length && keywords.length < 25; k++) {
              var kw = spl[k].trim();
              if (kw && keywords.indexOf(kw) === -1) keywords.push(kw);
            }
          }
        }
        var tagNodes = document.querySelectorAll(
          'meta[property="og:video:tag"]',
        );
        for (var t = 0; t < tagNodes.length && keywords.length < 25; t++) {
          var tagVal = String(tagNodes[t].getAttribute('content') || '').trim();
          if (tagVal && keywords.indexOf(tagVal) === -1) keywords.push(tagVal);
        }
      } catch (e) {}
    }

    return {
      title: title,
      category: category,
      author: author,
      keywords: keywords,
      hashtags: hashtags,
    };
  }

  async function blockThisPage(
    href,
    keys,
    unidentified,
    member,
    videoId,
    videoMeta,
  ) {
    stopVideo();
    var list = keys || [];
    await sendMessage({
      type: 'youtube_block',
      url: href,
      videoId: videoId || '',
      channel: list[0] || null,
      channels: list,
      unidentified: Boolean(unidentified),
      watchLaterMember: member,
      videoMeta: videoMeta || null,
    });
  }

  // Identifies the policy rules that can change a video decision, so a
  // repeated evaluation of an already-allowed video is skipped instead
  // of covering it again.
  function policyFingerprint(policy) {
    return JSON.stringify([
      policy.mode,
      policy.domains || [],
      policy.youtube_study_mode || false,
      policy.youtube_block_entertainment || false,
      policy.youtube_blocked_channels || [],
      policy.youtube_allowed_channels || [],
      policy.youtube_allow_watch_later || false,
    ]);
  }

  async function evaluate() {
    if (evaluating) {
      reevaluateRequested = true;
      return;
    }
    evaluating = true;
    try {
      var href = location.href;
      var url;
      try {
        url = new URL(href);
      } catch (e) {
        return;
      }
      if (!isYouTubeHost(url.hostname)) return;

      if (href !== lastUrl) {
        if (lastUrl !== '') isSpaNavigation = true;
        lastUrl = href;
        lastVideoId = videoIdFromVideoUrl(href) || '';
        lastOwnerKey = null;
        lastEvaluatedBridgeKey = '';
        unverifiedSince = null;
        pendingCheck = false;
        allowedKey = null;
        bridgeVideo = null;
        clearRetryEvaluate();
        if (
          isVideoPage(url.pathname) &&
          isChannelRestrictedPolicy(currentPolicy, currentBrowsingClosed)
        ) {
          showGuard();
        }
        requestBridgeRead();
      }

      applyControlsStyles();

      var response = await sendMessage({ type: 'youtube_policy' });
      if (location.href !== href) return;
      var policy = response ? response.policy : null;
      var browsingClosed = Boolean(response && response.browsingClosed);
      currentPolicy = policy;
      currentBrowsingClosed = browsingClosed;

      if (!policy) {
        clearUi();
        clearRetryEvaluate();
        unverifiedSince = null;
        pendingCheck = false;
        allowedKey = null;
      } else if (policy.mode === 'offline') {
        clearRetryEvaluate();
        return; // DNR blocks everything.
      } else {
        // Watch Later browsing: the homepage and the general feeds go to
        // the playlist instead of a blocked page, so the only permitted
        // YouTube content is one click away.
        if (
          policy.youtube_allow_watch_later &&
          browsingClosed &&
          isHomeSurface(url.pathname)
        ) {
          clearRetryEvaluate();
          redirectToWatchLater();
          return;
        }
        // Fail closed before awaiting async page/channel checks on video or
        // Shorts routes so restricted channels cannot start playing on cold load.
        if (
          (isVideoPage(url.pathname) || isShortsPath(url.pathname)) &&
          isChannelRestrictedPolicy(policy, browsingClosed) &&
          !allowedKey
        ) {
          showGuard();
        }
        // Every route is decided, not just video pages: SPA navigation
        // makes no network request, so nothing else would catch it.
        var page = await sendMessage({ type: 'youtube_page_check', url: href });
        if (location.href !== href) return;
        if (page && page.blocked) {
          pendingCheck = false;
          clearRetryEvaluate();
          await blockThisPage(href, [], false, null, '');
          return;
        }
        applyStyles(policy, browsingClosed);
      }

      // User YouTube Control: Redirect to Subscriptions (never overrides
      // Watch Later or policy blocks, which ran above).
      var subTarget = subscriptionsRedirectTarget(
        href,
        currentControls,
        policy,
        browsingClosed,
      );
      if (subTarget) {
        if (policy) {
          var subCheck = await sendMessage({
            type: 'youtube_page_check',
            url: subTarget,
          });
          if (location.href !== href) return;
          if (!subCheck || !subCheck.blocked) {
            redirectToSubscriptions(subTarget);
            return;
          }
        } else {
          redirectToSubscriptions(subTarget);
          return;
        }
      }

      // User YouTube Control: Direct Shorts URL restriction.
      if (
        isShortsPath(url.pathname) &&
        currentControls &&
        currentControls.recommendations.hideShorts
      ) {
        showShortsCover();
      } else {
        hideShortsCover();
      }

      applyDynamicDomControls();

      if (!policy) {
        hideGuard();
        pendingCheck = false;
        clearRetryEvaluate();
        return;
      }

      // Initial-load channel pages (/@handle, /c/name, /user/name): if a
      // policy blocks by UC ID before aliases were learned, check static
      // initial <meta itemprop="channelId"> / canonical channel link once.
      if (!isVideoPage(url.pathname)) {
        hideGuard();
        pendingCheck = false;
        clearRetryEvaluate();
        var pageChannelKey = parseChannelKeyFromPath(url.pathname);
        if (
          pageChannelKey &&
          !isSpaNavigation &&
          isChannelRestrictedPolicy(policy, browsingClosed)
        ) {
          var metaChannelHrefs = [];
          try {
            var metaIdNode =
              document.querySelector('meta[itemprop="identifier"]') ||
              document.querySelector('meta[itemprop="channelId"]');
            var metaCid =
              metaIdNode &&
              metaIdNode.getAttribute &&
              metaIdNode.getAttribute('content');
            if (metaCid && validUcId(metaCid)) {
              metaChannelHrefs.push('/channel/' + metaCid);
            }
          } catch (e) {}
          var pageChannelKeys = collectChannelKeys(
            metaChannelHrefs.concat([href]),
          );
          if (pageChannelKeys.length > 1) {
            var chanDecision = await sendMessage({
              type: 'youtube_check',
              url: href,
              channel: pageChannelKeys[0] || null,
              channels: pageChannelKeys,
              unidentified: false,
            });
            if (location.href !== href) return;
            if (chanDecision && chanDecision.blocked) {
              await blockThisPage(href, pageChannelKeys, false, null, '');
            }
          }
        }
        return;
      }

      var urlVid = videoIdFromVideoUrl(href);
      var activeBridge =
        bridgeVideo &&
        (!urlVid ||
          !bridgeVideo.videoId ||
          bridgeVideo.videoId === urlVid ||
          bridgeVideo.playerSwapped)
          ? bridgeVideo
          : null;
      var currentVideoId =
        activeBridge && activeBridge.playerSwapped && activeBridge.videoId
          ? activeBridge.videoId
          : urlVid || (activeBridge && activeBridge.videoId) || '';
      if (currentVideoId && lastVideoId && currentVideoId !== lastVideoId) {
        allowedKey = null;
        unverifiedSince = null;
      }
      if (currentVideoId) lastVideoId = currentVideoId;

      var channelPolicyActive = isChannelRestrictedPolicy(
        policy,
        browsingClosed,
      );
      if (channelPolicyActive && !allowedKey) {
        // Fail closed immediately before async identity resolution.
        showGuard();
      }

      var consumedBridgeKey = activeBridge
        ? (activeBridge.videoId || '') +
          '|' +
          (activeBridge.channelId || '') +
          '|' +
          (activeBridge.ownerUrl || '') +
          '|' +
          (activeBridge.category || '') +
          '|' +
          (activeBridge.title || '') +
          '|' +
          (activeBridge.playerSwapped ? '1' : '0')
        : '';

      var videoMeta = collectVideoMeta(activeBridge);
      lastMetaKey = (videoMeta.category || '') + '|' + (videoMeta.title || '');

      var rawOwnerHrefs = collectOwnerHrefs(currentVideoId);
      var keys = collectChannelKeys(rawOwnerHrefs);
      var hasHandle = keys.some(function (k) {
        return k.charAt(0) === '@';
      });
      if (!keys.length || (!hasHandle && currentVideoId)) {
        var oembedKey = await channelFromOEmbed(href, currentVideoId);
        if (location.href !== href) return;
        if (oembedKey && keys.indexOf(oembedKey) === -1) {
          keys = keys.concat([oembedKey]);
        }
      }
      if (keys.length) lastOwnerKey = keys.join(',');

      var watchLaterRoute = Boolean(
        policy.youtube_allow_watch_later && isWatchLaterVideoUrl(href),
      );
      var member = null;
      if (watchLaterRoute) {
        var ids = rememberWlIds();
        member = watchLaterMembership(
          currentVideoId || videoIdFromUrl(href),
          ids,
        );
      }

      var hasBlockedChannels = Boolean(
        (policy.youtube_blocked_channels || []).length,
      );
      var needsChannel =
        Boolean(policy.youtube_study_mode || hasBlockedChannels) &&
        !keys.length;
      var needsMember = watchLaterRoute && browsingClosed && member !== true;
      var needsMeta =
        Boolean(policy.youtube_block_entertainment) &&
        !(videoMeta && (videoMeta.category || videoMeta.title));
      if (needsChannel || needsMember || needsMeta) {
        if (unverifiedSince === null) unverifiedSince = Date.now();
      } else {
        unverifiedSince = null;
      }
      var timedOut =
        unverifiedSince !== null &&
        Date.now() - unverifiedSince >= DETECTION_TIMEOUT_MS;
      // Fail closed only after the window: a slow-loading panel must
      // not block a legitimate Watch Later video.
      if (needsMember && timedOut) member = false;

      var key =
        policyFingerprint(policy) +
        '|' +
        href +
        '|' +
        currentVideoId +
        '|' +
        keys.join(',') +
        '|' +
        String(member) +
        '|' +
        (videoMeta.category || '') +
        '|' +
        (videoMeta.title || '');
      if (
        key === allowedKey &&
        !pendingCheck &&
        (!consumedBridgeKey || consumedBridgeKey === lastEvaluatedBridgeKey)
      ) {
        clearRetryEvaluate();
        hideGuard();
        return;
      }

      if (channelPolicyActive && key !== allowedKey) showGuard();
      var checkUnidentified = Boolean((needsChannel || needsMeta) && timedOut);
      var decision = await sendMessage({
        type: 'youtube_check',
        url: href,
        videoId: currentVideoId || '',
        channel: keys[0] || null,
        channels: keys,
        unidentified: checkUnidentified,
        watchLaterMember: member,
        videoMeta: videoMeta,
      });
      if (location.href !== href) return;
      if (
        decision &&
        decision.videoId &&
        currentVideoId &&
        decision.videoId !== currentVideoId
      ) {
        reevaluateRequested = true;
        return;
      }
      if (!decision) {
        if (channelPolicyActive) scheduleRetryEvaluate();
        else {
          clearRetryEvaluate();
          hideGuard();
        }
        return;
      }
      if (decision.blocked) {
        pendingCheck = false;
        clearRetryEvaluate();
        lastEvaluatedBridgeKey = consumedBridgeKey;
        await blockThisPage(
          href,
          keys,
          checkUnidentified,
          member,
          currentVideoId,
          videoMeta,
        );
        return;
      }
      if (decision.pending) {
        // Keep the neutral cover up and look again as the page fills in.
        pendingCheck = true;
        scheduleRetryEvaluate();
        return;
      }
      if (
        !policy.youtube_block_entertainment ||
        (videoMeta && (videoMeta.category || videoMeta.title))
      ) {
        allowedKey = key;
      }
      lastEvaluatedBridgeKey = consumedBridgeKey;
      pendingCheck = false;
      clearRetryEvaluate();
      hideGuard();
    } finally {
      evaluating = false;
      if (reevaluateRequested) {
        reevaluateRequested = false;
        setTimeout(function () {
          void evaluate();
        }, 0);
      }
    }
  }

  // Re-evaluate only when something that could change the decision
  // changed: the URL, the owner links, or a verification still in
  // flight. YouTube mutates the DOM constantly (progress bar,
  // comments), so an unguarded observer would spam the worker.
  var lastMetaKey = '';
  var pending = false;
  function scheduleEvaluate() {
    if (pending) return;
    pending = true;
    setTimeout(function () {
      pending = false;
      var href = location.href;
      var checkVid =
        bridgeVideo && bridgeVideo.playerSwapped && bridgeVideo.videoId
          ? bridgeVideo.videoId
          : videoIdFromVideoUrl(href);
      var ownerKey = collectChannelKeys(collectOwnerHrefs(checkVid)).join(',');
      var currentMeta = collectVideoMeta(bridgeVideo);
      var currentMetaKey =
        (currentMeta.category || '') + '|' + (currentMeta.title || '');
      if (
        pendingCheck ||
        href !== lastUrl ||
        ownerKey !== lastOwnerKey ||
        currentMetaKey !== lastMetaKey
      ) {
        void evaluate();
      } else {
        scheduleDynamicControls();
      }
    }, 250);
  }

  async function evaluateInlinePreview(msg) {
    var vid = typeof msg.videoId === 'string' ? msg.videoId : '';
    if (!vid) return;
    var previewKey =
      vid +
      '|' +
      (typeof msg.channelId === 'string' ? msg.channelId : '') +
      '|' +
      (typeof msg.ownerUrl === 'string' ? msg.ownerUrl : '');
    if (previewKey === currentPreviewKey && !currentPreviewBlocked) return;
    currentPreviewKey = previewKey;
    currentPreviewBlocked = false;
    if (
      !currentPolicy ||
      !isChannelRestrictedPolicy(currentPolicy, currentBrowsingClosed)
    ) {
      return;
    }
    var raw = [];
    if (msg.channelId && validUcId(msg.channelId))
      raw.push('/channel/' + msg.channelId);
    if (msg.ownerUrl) raw.push(msg.ownerUrl);
    var keys = collectChannelKeys(raw);
    if (!keys.length) return;
    var check = await sendMessage({
      type: 'youtube_check',
      url: 'https://www.youtube.com/watch?v=' + encodeURIComponent(vid),
      videoId: vid,
      channel: keys[0] || null,
      channels: keys,
      unidentified: false,
      watchLaterMember: null,
    });
    if (currentPreviewKey !== previewKey) return;
    if (check && check.blocked) {
      currentPreviewBlocked = true;
      try {
        var pv = document.querySelector('#inline-preview-player video');
        if (pv) protectPlayback(pv);
      } catch (e) {}
    }
  }

  // Receive verified player identity from page-bridge.js (MAIN world).
  window.addEventListener('message', function (event) {
    if (event.source !== window || event.origin !== location.origin) return;
    var msg = event.data;
    if (!msg || msg.source !== 'preelock-page') return;
    if (msg.type === 'nav') {
      bridgeVideo = null;
      allowedKey = null;
      lastOwnerKey = null;
      lastEvaluatedBridgeKey = '';
      unverifiedSince = null;
      clearRetryEvaluate();
      if (lastUrl !== '') isSpaNavigation = true;
      if (
        isVideoPage(location.pathname) &&
        isChannelRestrictedPolicy(currentPolicy, currentBrowsingClosed)
      ) {
        stopVideo();
        showGuard();
      }
      void evaluate();
      return;
    }
    if (msg.type === 'preview-video') {
      void evaluateInlinePreview(msg);
      return;
    }
    if (msg.type !== 'video') return;
    var vid = typeof msg.videoId === 'string' ? msg.videoId : '';
    var currentVid = videoIdFromVideoUrl(location.href);
    var playerSwapped = Boolean(msg.playerSwapped);
    if (currentVid && vid && vid !== currentVid && !playerSwapped) return;
    var cid = typeof msg.channelId === 'string' ? msg.channelId : '';
    var ourl = typeof msg.ownerUrl === 'string' ? msg.ownerUrl : '';
    var title = typeof msg.title === 'string' ? msg.title : '';
    var category = typeof msg.category === 'string' ? msg.category : '';
    var keywords = Array.isArray(msg.keywords) ? msg.keywords : [];
    var msgKey =
      vid +
      '|' +
      cid +
      '|' +
      ourl +
      '|' +
      category +
      '|' +
      title +
      '|' +
      (playerSwapped ? '1' : '0');
    if (playerSwapped || (vid && lastVideoId && vid !== lastVideoId)) {
      allowedKey = null;
      lastOwnerKey = null;
      unverifiedSince = null;
      if (isChannelRestrictedPolicy(currentPolicy, currentBrowsingClosed)) {
        showGuard();
      }
    }
    bridgeVideo = {
      videoId: vid,
      channelId: cid,
      ownerUrl: ourl,
      title: title,
      category: category,
      keywords: keywords,
      playerSwapped: playerSwapped,
    };
    if (
      pendingCheck ||
      !allowedKey ||
      location.href !== lastUrl ||
      msgKey !== lastEvaluatedBridgeKey ||
      playerSwapped
    ) {
      void evaluate();
    }
  });

  // Guard any <video> that attempts to play while unverified, transitioning,
  // previewing a blocked channel, or Shorts-hidden.
  function handleMediaAttempt(event) {
    var target = event && event.target;
    if (!target || target.tagName !== 'VIDEO') return;
    if (isInlinePreviewVideo(target)) {
      if (currentPreviewBlocked) protectPlayback(target);
      return;
    }
    var routeChanged = location.href !== lastUrl;
    if (
      (routeChanged &&
        isVideoPage(location.pathname) &&
        isChannelRestrictedPolicy(currentPolicy, currentBrowsingClosed)) ||
      guardActive ||
      (currentControls &&
        currentControls.recommendations.hideShorts &&
        isShortsPath(location.pathname))
    ) {
      if (routeChanged) {
        bridgeVideo = null;
        allowedKey = null;
        lastOwnerKey = null;
        lastEvaluatedBridgeKey = '';
        if (isChannelRestrictedPolicy(currentPolicy, currentBrowsingClosed)) {
          showGuard();
        }
        void evaluate();
      }
      protectPlayback(target);
    }
  }

  if (typeof document.addEventListener === 'function') {
    document.addEventListener('play', handleMediaAttempt, true);
    document.addEventListener('playing', handleMediaAttempt, true);
    document.addEventListener('loadedmetadata', handleMediaAttempt, true);
    document.addEventListener(
      'ended',
      function (event) {
        var target = event && event.target;
        if (!target || target.tagName !== 'VIDEO') return;
        enforceDisableAutoplay();
      },
      true,
    );
  }

  window.addEventListener('yt-navigate-start', function () {
    bridgeVideo = null;
    allowedKey = null;
    lastOwnerKey = null;
    lastEvaluatedBridgeKey = '';
    unverifiedSince = null;
    clearRetryEvaluate();
    if (lastUrl !== '') isSpaNavigation = true;
    if (isChannelRestrictedPolicy(currentPolicy, currentBrowsingClosed)) {
      stopVideo();
      if (isVideoPage(location.pathname)) showGuard();
    }
  });
  [
    'yt-navigate-finish',
    'yt-page-data-updated',
    'yt-player-updated',
    'popstate',
    'hashchange',
  ].forEach(function (name) {
    window.addEventListener(name, function () {
      void evaluate();
    });
  });

  function attachBodyObserver() {
    if (typeof MutationObserver === 'undefined') return;
    if (document.body) {
      new MutationObserver(function (mutations) {
        var relevant = false;
        for (var i = 0; i < mutations.length; i++) {
          var target = mutations[i].target;
          if (
            target &&
            target.id &&
            (target.id === GUARD_ID || target.id === SHORTS_COVER_ID)
          )
            continue;
          relevant = true;
          break;
        }
        if (relevant) scheduleEvaluate();
      }).observe(document.body, {
        childList: true,
        subtree: true,
      });
      return;
    }
    if (document.documentElement) {
      var bootObserver = new MutationObserver(function () {
        if (document.body) {
          bootObserver.disconnect();
          attachBodyObserver();
          attachTitleObserver();
        }
      });
      bootObserver.observe(document.documentElement, { childList: true });
    }
  }

  function attachTitleObserver() {
    if (typeof MutationObserver === 'undefined') return;
    try {
      var titleEl = document.querySelector('title');
      if (titleEl) {
        new MutationObserver(function () {
          scheduleEvaluate();
        }).observe(titleEl, {
          childList: true,
          characterData: true,
          subtree: true,
        });
      } else if (document.head) {
        new MutationObserver(function () {
          var t = document.querySelector('title');
          if (t) {
            scheduleEvaluate();
          }
        }).observe(document.head, { childList: true });
      }
    } catch (e) {}
  }
  attachBodyObserver();
  attachTitleObserver();

  // Load persisted YouTube Controls immediately and listen for live popup toggles.
  if (
    controlsApi &&
    typeof chrome !== 'undefined' &&
    chrome.storage &&
    chrome.storage.local
  ) {
    chrome.storage.local.get(controlsApi.STORAGE_KEY, function (res) {
      if (chrome.runtime && chrome.runtime.lastError) return;
      currentControls = controlsApi.normalizeYoutubeControls(
        res ? res[controlsApi.STORAGE_KEY] : null,
      );
      applyControlsStyles();
      applyDynamicDomControls();
      syncBridgeAutoplay(currentControls.player.disableAutoplay);
      void evaluate();
    });
    if (chrome.storage.onChanged) {
      chrome.storage.onChanged.addListener(function (changes, areaName) {
        if (areaName !== 'local') return;
        var change = changes[controlsApi.STORAGE_KEY];
        if (!change) return;
        currentControls = controlsApi.normalizeYoutubeControls(change.newValue);
        applyControlsStyles();
        applyDynamicDomControls();
        syncBridgeAutoplay(currentControls.player.disableAutoplay);
        void evaluate();
      });
    }
  }

  // Playback sampling for the desktop app's lecture exemption. The
  // worker asks the active tab for its measured playback state; nothing here is
  // sent anywhere else, and no URL, title or query is involved.
  var media = { playing: false, sampledAt: 0, time: -1 };
  function sampleMedia() {
    var now = Date.now();
    var video = null;
    var url = null;
    try {
      video =
        document.querySelector('#movie_player video') ||
        document.querySelector('video');
      url = new URL(location.href);
    } catch (e) {
      video = null;
    }
    if (!video || !url) {
      media = { playing: false, sampledAt: now, time: -1 };
      return;
    }
    var time = Number(video.currentTime);
    if (!Number.isFinite(time)) time = -1;
    var advanced = media.time >= 0 && time > media.time + 0.1;
    var adShowing = false;
    try {
      adShowing = Boolean(
        document.querySelector('#movie_player.ad-showing') ||
        document.querySelector('.ad-showing'),
      );
    } catch (e) {}
    var playing = lecturePlaying({
      lecturePage: isLectureHost(url.hostname) && isLecturePath(url.pathname),
      adShowing: adShowing,
      paused: Boolean(video.paused),
      ended: Boolean(video.ended),
      currentTime: time,
      duration: Number(video.duration),
      visible: document.visibilityState !== 'hidden',
      playheadAdvanced: advanced,
    });
    media = { playing: playing, sampledAt: now, time: time };
  }
  // A sample older than a few seconds proves nothing — a throttled or
  // background tab must never keep reporting playback.
  function mediaPlaying() {
    return media.playing && Date.now() - media.sampledAt <= 5000;
  }
  chrome.runtime.onMessage.addListener(
    function (message, sender, sendResponse) {
      if (message && message.type === 'preelock_media_state') {
        sendResponse({ playing: mediaPlaying() });
        return;
      }
      if (message && message.type === 'preelock_policy_changed')
        void evaluate();
    },
  );
  setInterval(sampleMedia, 1000);
  setInterval(function () {
    void evaluate();
  }, POLL_MS);
  applyControlsStyles();
  sampleMedia();
  void evaluate();
})();
