// YouTube watch-page enforcement (step 3). A watch page URL carries
// no channel, so this content script reads the channel from the page
// DOM (owner-chip links to /channel/UC... and /@handle) and asks the
// worker for the decision (worker.js youtube_check), which owns the
// policy and the precedence rules. Channel pages need no help here:
// DNR rules (rules.js compileRules) block them. The script never
// sends a URL, title or query anywhere: the worker decision uses the
// page URL only inside the extension, and the heartbeat carries only
// the channel_blocked boolean.
//
// YouTube is a single-page app, so evaluation runs on load, on
// yt-navigate-finish / popstate, on owner-area DOM changes, on the
// worker's preelock_policy_changed message (session start/stop while
// the tab is open), and on a 5 s poll (a tab that was already open
// when the session started gets no navigation event).
//
// Fail behavior: block-list mode is fail-OPEN (only a positively
// identified channel is blocked; an unidentified channel plays).
// Study mode is fail-CLOSED on video pages, but only after a short
// detection timeout: once a player is loaded and 5 s have passed with
// still no channel found, the page is blocked with an explanation
// (the worker sends the tab to blocked.html). All rules expire with
// the session policy lease; with no policy this script does nothing.
//
// The pure functions below are the testable core (see
// youtube.test.js, which loads this file and calls them). They mirror
// rules.js (channelKeyFromPath / channelKeyFromHref /
// extractChannelKey) — the content script is a classic script and
// cannot import the worker's ES module, so the small parsers are
// duplicated here and kept in sync by test.
(function () {
  'use strict';
  var DETECTION_TIMEOUT_MS = 5000;
  var POLL_MS = 5000;

  function isYouTubeHost(hostname) {
    var host = String(hostname || '').toLowerCase();
    return host === 'youtube.com' || host.endsWith('.youtube.com');
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
      name.length >= 1 &&
      name.length <= 100 &&
      /^[a-z0-9._-]+$/.test(name)
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
    return false;
  }

  // Exposed for tests (and harmless in the isolated content-script
  // world, which the page itself cannot see).
  if (typeof globalThis !== 'undefined') {
    globalThis.__preelockYoutube = {
      DETECTION_TIMEOUT_MS: DETECTION_TIMEOUT_MS,
      isYouTubeHost: isYouTubeHost,
      parseChannelKeyFromPath: parseChannelKeyFromPath,
      parseChannelKeyFromHref: parseChannelKeyFromHref,
      extractChannelKey: extractChannelKey,
      isVideoPage: isVideoPage,
    };
  }

  // No extension context (tests, or a page that somehow loaded this
  // file directly): define the pure functions and stop.
  if (typeof chrome === 'undefined' || !chrome.runtime || !chrome.runtime.id)
    return;

  var lastUrl = '';
  var lastOwnerKey = null;
  var noChannelSince = null;
  var evaluating = false;

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
  function collectOwnerHrefs() {
    var hrefs = [];
    var seen = {};
    function add(href) {
      if (href && !seen[href]) {
        seen[href] = true;
        hrefs.push(href);
      }
    }
    var nodes;
    try {
      nodes = document.querySelectorAll(
        '#owner a[href], ytd-channel-name a[href], #channel-name a[href], ' +
          'a[href^="/@"], a[href^="/channel/"], ' +
          'a[href*="youtube.com/@"], a[href*="youtube.com/channel/"]',
      );
    } catch (e) {
      nodes = [];
    }
    for (var i = 0; i < nodes.length; i++) {
      var href = nodes[i].getAttribute
        ? nodes[i].getAttribute('href')
        : nodes[i].href;
      add(href);
    }
    // Fallback: YouTube embeds the channel in meta tags, which are present
    // even when the owner chip hasn't rendered yet (SPA navigation).
    try {
      var meta = document.querySelector('meta[itemprop="channelId"]');
      var cid = meta && meta.getAttribute('content');
      if (cid) add('/channel/' + cid);
      var link = document.querySelector('link[itemprop="url"]');
      var lhref = link && link.getAttribute('href');
      if (lhref) add(lhref);
    } catch (e) {}
    return hrefs;
  }
  function stopVideo() {
    try {
      var video = document.querySelector('video');
      if (video && video.pause) video.pause();
    } catch (e) {}
  }
  async function evaluate() {
    if (evaluating) return;
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
        lastUrl = href;
        lastOwnerKey = null;
        noChannelSince = null;
      }
      if (!isVideoPage(url.pathname)) {
        noChannelSince = null;
        return;
      }
      var policyResponse = await sendMessage({ type: 'youtube_policy' });
      var policy = policyResponse ? policyResponse.policy : null;
      if (!policy) {
        noChannelSince = null;
        return;
      }
      if (policy.mode === 'offline') return; // DNR blocks everything.
      var channel = extractChannelKey(collectOwnerHrefs());
      // Fallback: if DOM didn't yield a channel, ask YouTube's oEmbed API.
      // This is reliable on SPA navigation where the owner chip loads late.
      if (!channel) {
        try {
          var oembedUrl = 'https://www.youtube.com/oembed?url=' + encodeURIComponent(href) + '&format=json';
          var resp = await fetch(oembedUrl);
          if (resp && resp.ok) {
            var data = await resp.json();
            if (data && data.author_url) {
              channel = parseChannelKeyFromHref(data.author_url);
            }
          }
        } catch (e) {}
      }
      if (channel) {
        noChannelSince = null;
        lastOwnerKey = channel;
        var decision = await sendMessage({
          type: 'youtube_check',
          url: href,
          channel: channel,
        });
        if (decision && decision.blocked) {
          stopVideo();
          await sendMessage({ type: 'youtube_block' });
        }
        return;
      }
      // No channel found yet. Block-list mode is fail-OPEN: play.
      if (!policy.youtube_study_mode) {
        noChannelSince = null;
        return;
      }
      // Study mode is fail-CLOSED, but only after the detection
      // timeout with a loaded player: a slow-loading owner chip must
      // not block an allowed channel's video.
      var player = null;
      try {
        player = document.querySelector('video, #movie_player');
      } catch (e) {}
      if (!player) return;
      if (noChannelSince === null) noChannelSince = Date.now();
      if (Date.now() - noChannelSince < DETECTION_TIMEOUT_MS) return;
      var timeoutDecision = await sendMessage({
        type: 'youtube_check',
        url: href,
        channel: null,
        unidentified: true,
      });
      if (timeoutDecision && timeoutDecision.blocked) {
        stopVideo();
        await sendMessage({ type: 'youtube_block' });
      }
    } finally {
      evaluating = false;
    }
  }
  // Re-evaluate only when something that could change the decision
  // changed: the URL, or the owner links. YouTube mutates the DOM
  // constantly (progress bar, comments), so an unguarded observer
  // would spam the worker.
  var pending = false;
  function scheduleEvaluate() {
    if (pending) return;
    pending = true;
    setTimeout(function () {
      pending = false;
      var href = location.href;
      var ownerKey = extractChannelKey(collectOwnerHrefs());
      if (href !== lastUrl || ownerKey !== lastOwnerKey) void evaluate();
    }, 500);
  }
  window.addEventListener('yt-navigate-finish', function () {
    void evaluate();
  });
  window.addEventListener('popstate', function () {
    void evaluate();
  });
  if (typeof MutationObserver !== 'undefined' && document.body) {
    new MutationObserver(scheduleEvaluate).observe(document.body, {
      childList: true,
      subtree: true,
    });
  }
  chrome.runtime.onMessage.addListener(function (message) {
    if (message && message.type === 'preelock_policy_changed') void evaluate();
  });
  setInterval(function () {
    void evaluate();
  }, POLL_MS);
  void evaluate();
})();
