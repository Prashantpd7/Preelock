// Page-world bridge (MAIN world, document_start). The content script
// lives in an isolated world and cannot read YouTube's player object,
// which is the only source that names the *currently playing* video's
// channel before the DOM owner chip has re-rendered. This script reads
// that object and forwards nothing but the current video identity to
// the isolated world through window.postMessage.
//
// It trusts nothing it reads: every field is re-validated in the
// isolated world (video id shape, channel id shape, owner URL parsed by
// the same channel parser), and the value is only used as an identity
// hint for the policy decision the worker makes. Page data can never
// allow a video by itself.
(function () {
  'use strict';
  if (window.__preelockYtBridge) return;
  window.__preelockYtBridge = true;
  var last = '';
  var lastPreview = '';
  var lastUrlHref = location.href;
  var settledForUrl = false;
  var settledVideoId = '';

  function currentUrlVideoId() {
    try {
      var u = new URL(location.href);
      if (u.pathname === '/watch' || u.pathname === '/watch/') {
        return u.searchParams.get('v') || '';
      }
      var parts = u.pathname.split('/').filter(Boolean);
      if ((parts[0] === 'shorts' || parts[0] === 'live') && parts[1]) {
        return parts[1];
      }
    } catch (e) {}
    return '';
  }

  function syncUrlState() {
    if (location.href !== lastUrlHref) {
      lastUrlHref = location.href;
      settledForUrl = false;
      settledVideoId = '';
      last = '';
    }
  }

  function playerResponse(player) {
    try {
      if (player && typeof player.getPlayerResponse === 'function')
        return player.getPlayerResponse() || null;
    } catch (e) {}
    return null;
  }

  function extractFromResponse(response) {
    if (!response || typeof response !== 'object') return null;
    var details = response.videoDetails || null;
    var micro =
      (response.microformat &&
        response.microformat.playerMicroformatRenderer) ||
      null;
    var videoId = (details && details.videoId) || '';
    var author = (details && details.author) || '';
    var channelId =
      (details && details.channelId) ||
      (micro && micro.externalChannelId) ||
      '';
    var ownerUrl =
      (micro && (micro.canonicalBaseUrl || micro.ownerProfileUrl)) || '';
    var title =
      (details && details.title) ||
      (micro &&
        micro.title &&
        (micro.title.simpleText ||
          (Array.isArray(micro.title.runs) &&
            micro.title.runs[0] &&
            micro.title.runs[0].text))) ||
      '';
    var category = (micro && micro.category) || '';
    var keywords =
      details && Array.isArray(details.keywords)
        ? details.keywords.slice(0, 25).map(String)
        : [];
    if (!videoId && !channelId && !ownerUrl && !author && !title && !category)
      return null;
    return {
      videoId: videoId,
      channelId: channelId,
      ownerUrl: ownerUrl,
      author: author,
      title: title,
      category: category,
      keywords: keywords,
    };
  }

  function extractFromFlexy(expectedId) {
    try {
      var flexy =
        document.querySelector('ytd-watch-flexy:not([hidden])') ||
        document.querySelector('ytd-watch-flexy');
      if (!flexy) return null;
      var attrId = flexy.getAttribute('video-id') || '';
      if (expectedId && attrId && attrId !== expectedId) return null;
      var pInfo = extractFromResponse(flexy.playerData);
      if (
        pInfo &&
        pInfo.videoId &&
        (!expectedId || pInfo.videoId === expectedId) &&
        (pInfo.channelId || pInfo.ownerUrl)
      ) {
        return pInfo;
      }
      var data = flexy.data || null;
      var endpointVid =
        data &&
        data.currentVideoEndpoint &&
        data.currentVideoEndpoint.watchEndpoint &&
        data.currentVideoEndpoint.watchEndpoint.videoId;
      var dataVid = endpointVid || attrId || '';
      if (!dataVid || (expectedId && dataVid !== expectedId)) return null;
      var contents =
        data &&
        data.contents &&
        data.contents.twoColumnWatchNextResults &&
        data.contents.twoColumnWatchNextResults.results &&
        data.contents.twoColumnWatchNextResults.results.results &&
        data.contents.twoColumnWatchNextResults.results.results.contents;
      if (!Array.isArray(contents)) return null;
      for (var i = 0; i < contents.length; i++) {
        var sec = contents[i] && contents[i].videoSecondaryInfoRenderer;
        var owner = sec && sec.owner && sec.owner.videoOwnerRenderer;
        var browse =
          owner &&
          owner.navigationEndpoint &&
          owner.navigationEndpoint.browseEndpoint;
        if (browse) {
          var runs = owner.title && owner.title.runs;
          var author =
            Array.isArray(runs) && runs[0] && runs[0].text ? runs[0].text : '';
          return {
            videoId: dataVid,
            channelId: browse.browseId || '',
            ownerUrl: browse.canonicalBaseUrl || '',
            author: author,
          };
        }
      }
    } catch (e) {}
    return null;
  }

  function candidatePlayers() {
    var selectors = [
      '#movie_player',
      '#shorts-player',
      'ytd-reel-video-renderer[is-active] #player',
      '#c4-player',
      'ytd-player .html5-video-player',
    ];
    var out = [];
    for (var i = 0; i < selectors.length; i++) {
      try {
        var el = document.querySelector(selectors[i]);
        if (el && el.id !== 'inline-preview-player' && out.indexOf(el) === -1) {
          out.push(el);
        }
      } catch (e) {}
    }
    return out;
  }

  function readActiveReelOwner(expectedId) {
    try {
      var reel = document.querySelector('ytd-reel-video-renderer[is-active]');
      if (!reel) return null;
      var data = reel.data || reel.__data || null;
      var resp =
        data &&
        (data.playerResponse || (data.data && data.data.playerResponse));
      var parsed = extractFromResponse(resp);
      if (
        parsed &&
        (!expectedId || !parsed.videoId || parsed.videoId === expectedId) &&
        (parsed.channelId || parsed.ownerUrl)
      ) {
        return parsed;
      }
      var rwe =
        data &&
        (data.reelWatchEndpoint || (data.data && data.data.reelWatchEndpoint));
      var rweVid = (rwe && rwe.videoId) || (data && data.videoId) || '';
      if (expectedId && rweVid && rweVid !== expectedId) return null;
      var header =
        data &&
        data.overlay &&
        data.overlay.reelPlayerOverlayRenderer &&
        data.overlay.reelPlayerOverlayRenderer
          .reelPlayerHeaderSupportedRenderers &&
        data.overlay.reelPlayerOverlayRenderer
          .reelPlayerHeaderSupportedRenderers.reelPlayerHeaderRenderer;
      var browse =
        header &&
        header.channelNavigationEndpoint &&
        header.channelNavigationEndpoint.browseEndpoint;
      if (browse && (browse.browseId || browse.canonicalBaseUrl)) {
        return {
          videoId: rweVid || expectedId || '',
          channelId: browse.browseId || '',
          ownerUrl: browse.canonicalBaseUrl || '',
          author: '',
        };
      }
    } catch (e) {}
    return null;
  }

  function readInlinePreview() {
    try {
      var preview = document.querySelector('#inline-preview-player');
      if (!preview) {
        lastPreview = '';
        return;
      }
      var info = extractFromResponse(playerResponse(preview));
      var vdata =
        typeof preview.getVideoData === 'function'
          ? preview.getVideoData()
          : null;
      var vid = (info && info.videoId) || (vdata && vdata.video_id) || '';
      if (!vid) return;
      var channelId = (info && info.channelId) || '';
      var ownerUrl = (info && info.ownerUrl) || '';
      var author = (info && info.author) || (vdata && vdata.author) || '';
      var pKey = vid + '|' + channelId + '|' + ownerUrl + '|' + author;
      if (pKey === lastPreview) return;
      lastPreview = pKey;
      window.postMessage(
        {
          source: 'preelock-page',
          type: 'preview-video',
          videoId: vid,
          channelId: channelId,
          ownerUrl: ownerUrl,
          author: author,
        },
        location.origin,
      );
    } catch (e) {}
  }

  function read(force) {
    syncUrlState();
    readInlinePreview();

    var expectedId = currentUrlVideoId();
    var players = candidatePlayers();
    var videoId = '';
    var channelId = '';
    var ownerUrl = '';
    var author = '';
    var title = '';
    var category = '';
    var keywords = [];
    var playerSwapped = false;

    for (var i = 0; i < players.length; i++) {
      var player = players[i];
      var info = extractFromResponse(playerResponse(player));
      var vdata = null;
      var fallbackId = '';
      try {
        vdata =
          typeof player.getVideoData === 'function'
            ? player.getVideoData()
            : null;
        fallbackId = (vdata && vdata.video_id) || '';
      } catch (e) {}
      var id = (info && info.videoId) || fallbackId;
      if (!id) continue;

      if (expectedId && id !== expectedId) {
        // Accept a player swap without URL change ONLY once this URL already
        // settled on another video and both playerResponse and videoData agree
        // on the new non-ad video ID + owner metadata.
        var consistentSwap =
          settledForUrl &&
          location.href === lastUrlHref &&
          settledVideoId &&
          info &&
          info.videoId &&
          info.videoId !== settledVideoId &&
          (!fallbackId || fallbackId === info.videoId) &&
          (info.channelId || info.ownerUrl);
        if (!consistentSwap) continue;
        playerSwapped = true;
      }

      videoId = id;
      channelId = (info && info.channelId) || '';
      ownerUrl = (info && info.ownerUrl) || '';
      author = (info && info.author) || (vdata && vdata.author) || '';
      title = (info && info.title) || (vdata && vdata.title) || '';
      category = (info && info.category) || '';
      keywords = (info && info.keywords) || [];
      if (channelId || ownerUrl) break;
    }

    if (
      !playerSwapped &&
      expectedId &&
      (!videoId || (!channelId && !ownerUrl))
    ) {
      var flexyInfo = extractFromFlexy(expectedId);
      if (flexyInfo) {
        videoId = videoId || flexyInfo.videoId || expectedId;
        channelId = channelId || flexyInfo.channelId;
        ownerUrl = ownerUrl || flexyInfo.ownerUrl;
        author = author || flexyInfo.author;
        title = title || flexyInfo.title || '';
        category = category || flexyInfo.category || '';
        if (!keywords.length && flexyInfo.keywords)
          keywords = flexyInfo.keywords;
      }
    }

    if (
      !playerSwapped &&
      expectedId &&
      (!videoId || (!channelId && !ownerUrl))
    ) {
      var reelInfo = readActiveReelOwner(expectedId);
      if (reelInfo) {
        videoId = videoId || reelInfo.videoId || expectedId;
        channelId = channelId || reelInfo.channelId;
        ownerUrl = ownerUrl || reelInfo.ownerUrl;
        author = author || reelInfo.author;
      }
    }

    if (
      !playerSwapped &&
      expectedId &&
      (!videoId || (!channelId && !ownerUrl) || !category || !title)
    ) {
      try {
        var initInfo = extractFromResponse(window.ytInitialPlayerResponse);
        if (initInfo && initInfo.videoId === expectedId) {
          videoId = videoId || initInfo.videoId;
          channelId = channelId || initInfo.channelId;
          ownerUrl = ownerUrl || initInfo.ownerUrl;
          author = author || initInfo.author;
          title = title || initInfo.title || '';
          category = category || initInfo.category || '';
          if (!keywords.length && initInfo.keywords)
            keywords = initInfo.keywords;
        }
      } catch (e) {}
    }

    if (!videoId) return;
    if (!playerSwapped && expectedId && videoId !== expectedId) return;

    settledForUrl = true;
    settledVideoId = videoId;

    var key =
      videoId +
      '|' +
      channelId +
      '|' +
      ownerUrl +
      '|' +
      author +
      '|' +
      category +
      '|' +
      title +
      '|' +
      (playerSwapped ? '1' : '0');
    if (!force && key === last) return;
    last = key;
    try {
      window.postMessage(
        {
          source: 'preelock-page',
          type: 'video',
          videoId: videoId,
          channelId: channelId,
          ownerUrl: ownerUrl,
          author: author,
          title: title,
          category: category,
          keywords: keywords,
          urlVideoId: expectedId,
          playerSwapped: playerSwapped,
        },
        location.origin,
      );
    } catch (e) {}
  }

  function applyAutoplay(disabled) {
    if (!disabled) return;
    try {
      var player = document.querySelector('#movie_player');
      if (player && typeof player.setAutonav === 'function') {
        player.setAutonav(false);
      }
      if (player && typeof player.setAutonavState === 'function') {
        player.setAutonavState(1);
      }
    } catch (e) {}
  }

  function notifyNav() {
    syncUrlState();
    last = '';
    try {
      window.postMessage(
        {
          source: 'preelock-page',
          type: 'nav',
          href: location.href,
        },
        location.origin,
      );
    } catch (e) {}
    read(true);
  }

  ['pushState', 'replaceState'].forEach(function (method) {
    try {
      var orig = history[method];
      if (typeof orig !== 'function') return;
      history[method] = function () {
        var ret = orig.apply(this, arguments);
        notifyNav();
        return ret;
      };
    } catch (e) {}
  });
  window.addEventListener('popstate', notifyNav, true);

  window.addEventListener('message', function (event) {
    if (event.source !== window || event.origin !== location.origin) return;
    var msg = event.data;
    if (!msg || msg.source !== 'preelock-content') return;
    if (msg.type === 'read-video') {
      read(true);
    } else if (msg.type === 'set-autoplay') {
      applyAutoplay(Boolean(msg.disabled));
    }
  });

  // The player's response changes when YouTube swaps videos, including
  // autoplay and playlist transitions where nothing else reliably tells
  // us in time. A short interval keeps this cheap and immediate.
  setInterval(function () {
    read(false);
  }, 200);
  for (var i = 0; i < 20; i++) {
    setTimeout(function () {
      read(false);
    }, i * 250);
  }
  window.addEventListener(
    'yt-navigate-start',
    function () {
      settledForUrl = false;
      settledVideoId = '';
      last = '';
      try {
        window.postMessage(
          {
            source: 'preelock-page',
            type: 'nav',
            href: location.href,
          },
          location.origin,
        );
      } catch (e) {}
    },
    true,
  );
  ['yt-navigate-finish', 'yt-player-updated', 'yt-page-data-updated'].forEach(
    function (name) {
      window.addEventListener(
        name,
        function () {
          last = '';
          read(true);
        },
        true,
      );
    },
  );
  document.addEventListener(
    'DOMContentLoaded',
    function () {
      read(false);
    },
    true,
  );
  read(false);
})();
