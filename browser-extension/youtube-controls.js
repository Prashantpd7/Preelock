// YouTube distraction-control schema, normalization, UI metadata and
// CSS rule generation shared by popup.js and youtube.js.
//
// Stored locally in chrome.storage.local under STORAGE_KEY so all user
// preferences persist across browser restarts and extension reloads
// without touching Preelock's desktop pairing or session policy keys.
(function (root) {
  'use strict';

  var STORAGE_KEY = 'youtubeControls';
  var SCHEMA_VERSION = 1;
  var CONTROLS_STYLE_ID = 'preelock-yt-controls-style';

  var DEFAULT_YOUTUBE_CONTROLS = {
    version: SCHEMA_VERSION,
    home: {
      hideHomeFeed: false,
      redirectToSubscriptions: false,
    },
    navigation: {
      hideExploreAndTrending: false,
      hideSubscriptions: false,
    },
    recommendations: {
      hideVideoSidebar: false,
      hideRecommended: false,
      hideLiveChat: false,
      hidePlaylist: false,
      hideFundraiser: false,
      hideEndScreenFeed: false,
      hideEndScreenCards: false,
      hideShorts: false,
      hideMixes: false,
    },
    player: {
      hideVideoInfo: false,
      hideDescription: false,
      hideMetadata: false,
      hideActionButtons: false,
      hideChannelInfo: false,
      disableAutoplay: false,
      disableAnnotations: false,
    },
    social: {
      hideComments: false,
      hideCommentTeaser: false,
      hideCommentReplies: false,
      hideNotifications: false,
    },
    other: {
      hideMerchTicketsOffers: false,
      hideTopHeader: false,
    },
  };

  // Popup hierarchy: 5 logical groups, with expandable sub-controls for
  // Video Sidebar, Video Info, Comments, and Top Header. Controls that
  // appear in two conceptual places (Hide Live Chat, Hide Notifications)
  // bind to the same canonical path so both stay synchronized.
  var YOUTUBE_CONTROL_GROUPS = [
    {
      id: 'home',
      title: 'Home and Navigation',
      defaultExpanded: false,
      controls: [
        { path: 'home.hideHomeFeed', label: 'Hide Home Feed' },
        {
          path: 'home.redirectToSubscriptions',
          label: 'Redirect to Subscriptions',
        },
        {
          path: 'navigation.hideExploreAndTrending',
          label: 'Hide Explore and Trending',
        },
        { path: 'navigation.hideSubscriptions', label: 'Hide Subscriptions' },
      ],
    },
    {
      id: 'recommendations',
      title: 'Video Recommendations',
      defaultExpanded: false,
      controls: [
        {
          path: 'recommendations.hideVideoSidebar',
          label: 'Hide Video Sidebar',
          children: [
            {
              path: 'recommendations.hideRecommended',
              label: 'Hide Recommended',
            },
            { path: 'recommendations.hideLiveChat', label: 'Hide Live Chat' },
            { path: 'recommendations.hidePlaylist', label: 'Hide Playlist' },
            {
              path: 'recommendations.hideFundraiser',
              label: 'Hide Fundraiser',
            },
          ],
        },
        {
          path: 'recommendations.hideEndScreenFeed',
          label: 'Hide End Screen Feed',
        },
        {
          path: 'recommendations.hideEndScreenCards',
          label: 'Hide End Screen Cards',
        },
        { path: 'recommendations.hideShorts', label: 'Hide Shorts' },
        { path: 'recommendations.hideMixes', label: 'Hide Mixes' },
      ],
    },
    {
      id: 'player',
      title: 'Video Player',
      defaultExpanded: false,
      controls: [
        {
          path: 'player.hideVideoInfo',
          label: 'Hide Video Info',
          children: [
            { path: 'player.hideDescription', label: 'Hide Description' },
            { path: 'player.hideMetadata', label: 'Hide Views & Date' },
            {
              path: 'player.hideActionButtons',
              label: 'Hide Action Buttons',
            },
            {
              path: 'player.hideChannelInfo',
              label: 'Hide Channel & Subscribe',
            },
          ],
        },
        { path: 'player.disableAutoplay', label: 'Disable Autoplay' },
        { path: 'player.disableAnnotations', label: 'Disable Annotations' },
      ],
    },
    {
      id: 'social',
      title: 'Social and Community',
      defaultExpanded: false,
      controls: [
        {
          path: 'social.hideComments',
          label: 'Hide Comments',
          children: [
            { path: 'social.hideCommentTeaser', label: 'Hide Comment Teaser' },
            {
              path: 'social.hideCommentReplies',
              label: 'Hide Comment Replies',
            },
          ],
        },
        { path: 'recommendations.hideLiveChat', label: 'Hide Live Chat' },
        { path: 'social.hideNotifications', label: 'Hide Notifications' },
      ],
    },
    {
      id: 'other',
      title: 'Other YouTube Elements',
      defaultExpanded: false,
      controls: [
        {
          path: 'other.hideMerchTicketsOffers',
          label: 'Hide Merch, Tickets and Offers',
        },
        {
          path: 'other.hideTopHeader',
          label: 'Hide Top Header',
          children: [
            { path: 'social.hideNotifications', label: 'Hide Notifications' },
          ],
        },
      ],
    },
  ];

  function cloneDefaults() {
    var copy = { version: SCHEMA_VERSION };
    var sections = Object.keys(DEFAULT_YOUTUBE_CONTROLS);
    for (var i = 0; i < sections.length; i++) {
      var section = sections[i];
      if (section === 'version') continue;
      copy[section] = {};
      var keys = Object.keys(DEFAULT_YOUTUBE_CONTROLS[section]);
      for (var j = 0; j < keys.length; j++) {
        var key = keys[j];
        copy[section][key] = DEFAULT_YOUTUBE_CONTROLS[section][key];
      }
    }
    return copy;
  }

  // Validate and migrate stored settings safely so missing or malformed
  // fields always fall back to defaults without breaking existing state.
  function normalizeYoutubeControls(raw) {
    var normalized = cloneDefaults();
    if (!raw || typeof raw !== 'object') return normalized;
    var sections = Object.keys(DEFAULT_YOUTUBE_CONTROLS);
    for (var i = 0; i < sections.length; i++) {
      var section = sections[i];
      if (section === 'version') continue;
      var rawSection = raw[section];
      if (!rawSection || typeof rawSection !== 'object') continue;
      var keys = Object.keys(DEFAULT_YOUTUBE_CONTROLS[section]);
      for (var j = 0; j < keys.length; j++) {
        var key = keys[j];
        if (typeof rawSection[key] === 'boolean') {
          normalized[section][key] = rawSection[key];
        }
      }
    }
    // "Redirect to Subscriptions" and "Hide Subscriptions" are mutually
    // exclusive: redirecting home to Subscriptions while the Subscriptions
    // nav link is hidden strands the user on a page they cannot reach
    // from the nav. If stored state has both on (older version), Redirect
    // wins so both can never be applied at once.
    if (
      normalized.home.redirectToSubscriptions &&
      normalized.navigation.hideSubscriptions
    ) {
      normalized.navigation.hideSubscriptions = false;
    }
    return normalized;
  }

  function getControlValue(controls, path) {
    var normalized = normalizeYoutubeControls(controls);
    var parts = String(path || '').split('.');
    if (parts.length !== 2) return false;
    var section = normalized[parts[0]];
    return Boolean(section && section[parts[1]]);
  }

  function setControlValue(controls, path, value) {
    var next = normalizeYoutubeControls(controls);
    var parts = String(path || '').split('.');
    if (parts.length !== 2) return next;
    var sectionName = parts[0];
    var keyName = parts[1];
    if (
      next[sectionName] &&
      Object.prototype.hasOwnProperty.call(next[sectionName], keyName)
    ) {
      next[sectionName][keyName] = Boolean(value);
      // Mutual exclusion: enabling one of the Subscriptions pair turns
      // the other off; the last one enabled wins.
      if (Boolean(value)) {
        if (sectionName === 'home' && keyName === 'redirectToSubscriptions') {
          next.navigation.hideSubscriptions = false;
        } else if (
          sectionName === 'navigation' &&
          keyName === 'hideSubscriptions'
        ) {
          next.home.redirectToSubscriptions = false;
        }
      }
    }
    return next;
  }

  function hasActiveYoutubeControls(controls) {
    var normalized = normalizeYoutubeControls(controls);
    var sections = Object.keys(normalized);
    for (var i = 0; i < sections.length; i++) {
      var section = sections[i];
      if (section === 'version') continue;
      var keys = Object.keys(normalized[section]);
      for (var j = 0; j < keys.length; j++) {
        if (normalized[section][keys[j]] === true) return true;
      }
    }
    return false;
  }

  function countActiveControls(controls) {
    var normalized = normalizeYoutubeControls(controls);
    var count = 0;
    var sections = Object.keys(normalized);
    for (var i = 0; i < sections.length; i++) {
      var section = sections[i];
      if (section === 'version') continue;
      var keys = Object.keys(normalized[section]);
      for (var j = 0; j < keys.length; j++) {
        if (normalized[section][keys[j]] === true) count++;
      }
    }
    return count;
  }

  function isMainYouTubeHost(hostname) {
    var host = String(hostname || '').toLowerCase();
    return (
      host === 'youtube.com' ||
      host === 'www.youtube.com' ||
      host === 'm.youtube.com'
    );
  }

  function isShortsPath(pathname) {
    var path = String(pathname || '/').toLowerCase();
    return path === '/shorts' || path.startsWith('/shorts/');
  }

  function youtubeRouteKind(urlString) {
    var url;
    try {
      url = new URL(urlString || '', 'https://www.youtube.com/');
    } catch (e) {
      return 'other';
    }
    var path = String(url.pathname || '/').toLowerCase();
    if (path === '/') return 'home';
    if (
      path === '/feed/subscriptions' ||
      path.startsWith('/feed/subscriptions/')
    )
      return 'subscriptions';
    if (path === '/watch' || path === '/watch/') return 'watch';
    if (isShortsPath(path)) return 'shorts';
    if (path === '/live' || path.startsWith('/live/')) return 'live';
    if (path === '/results' || path.startsWith('/results/')) return 'search';
    if (path === '/playlist' || path === '/playlist/') return 'playlist';
    if (
      path.startsWith('/@') ||
      path.startsWith('/channel/') ||
      path.startsWith('/c/') ||
      path.startsWith('/user/')
    )
      return 'channel';
    return 'other';
  }

  // Extract a video ID from /watch?v=..., /shorts/<id>, or /live/<id>.
  // Kept separate from videoIdFromHref (which validates /watch?v= only
  // for Watch Later playlist verification).
  function videoIdFromVideoUrl(urlString) {
    if (typeof urlString !== 'string' || !urlString) return null;
    var url;
    try {
      url = new URL(urlString, 'https://www.youtube.com/');
    } catch (e) {
      return null;
    }
    var host = String(url.hostname || '').toLowerCase();
    if (host !== 'youtube.com' && !host.endsWith('.youtube.com')) return null;
    var path = url.pathname || '/';
    var id = null;
    if (path === '/watch' || path === '/watch/') {
      id = url.searchParams.get('v');
    } else {
      var parts = path.split('/').filter(Boolean);
      if ((parts[0] === 'shorts' || parts[0] === 'live') && parts[1]) {
        id = parts[1];
      }
    }
    return id && /^[A-Za-z0-9_-]{5,20}$/.test(id) ? id : null;
  }

  function controlsHiddenSelectors(rawControls) {
    var c = normalizeYoutubeControls(rawControls);
    var selectors = [];

    // A. Home and Navigation
    if (c.home.hideHomeFeed) {
      selectors.push(
        'html[data-preelock-route="home"] ytd-browse[page-subtype="home"] #contents',
        'html[data-preelock-route="home"] ytd-browse[page-subtype="home"] #header',
        'html[data-preelock-route="home"] ytd-browse[page-subtype="home"] ytd-rich-grid-renderer',
        'html[data-preelock-route="home"] ytd-browse[page-subtype="home"] ytd-feed-filter-chip-bar-renderer',
        'html[data-preelock-route="home"] ytm-rich-grid-renderer',
        'html[data-preelock-route="home"] ytm-feed-filter-chip-bar-renderer',
      );
    }

    if (c.navigation.hideExploreAndTrending) {
      selectors.push(
        'ytd-guide-entry-renderer:has(a[href^="/feed/trending"])',
        'ytd-guide-entry-renderer:has(a[href^="/feed/explore"])',
        'ytd-guide-entry-renderer:has(a[href="/gaming"])',
        'ytd-guide-entry-renderer:has(a[href^="/feed/storefront"])',
        'ytd-guide-entry-renderer:has(a[href="/podcasts"])',
        'ytd-guide-entry-renderer:has(a[href="/playables"])',
        'ytd-guide-entry-renderer:has(a[href^="/channel/UC-9-kyTW8ZkZNDHQJ6FgpwQ"])',
        'ytd-guide-entry-renderer:has(a[href^="/channel/UC4R8DWoMoI7CAwX8_LjQHig"])',
        'ytd-guide-entry-renderer:has(a[href^="/channel/UCYfdidRxbB8Qhf0Nx7ioOYw"])',
        'ytd-guide-entry-renderer:has(a[href^="/channel/UCEgdi0XIXXZ-qJOFPf4JSKw"])',
        'ytd-guide-entry-renderer:has(a[href^="/channel/UCkYQyvc_i9hXEo4xic9Hh2g"])',
        'ytd-guide-entry-renderer:has(a[href^="/channel/UCrpQ4p1Ql_hG8rKXIKM1MOQ"])',
        'ytd-guide-entry-renderer:has(a[href^="/channel/UCtFRv9O2AHqOZjjynzrv-xg"])',
        'ytd-guide-entry-renderer:has(a[href^="/channel/UCOpNcN46UbXVtpKMrmU4Abg"])',
        'ytd-mini-guide-entry-renderer:has(a[href^="/feed/trending"])',
        'ytd-mini-guide-entry-renderer:has(a[href^="/feed/explore"])',
        'ytd-guide-section-renderer:has(a[href="/gaming"]):not(:has(a[href="/"])):not(:has(a[href="/feed/subscriptions"])):not(:has(a[href="/playlist?list=WL"]))',
        'ytd-guide-section-renderer:has(a[href^="/feed/trending"]):not(:has(a[href="/"])):not(:has(a[href="/feed/subscriptions"])):not(:has(a[href="/playlist?list=WL"]))',
        'ytd-browse[page-subtype="trending"] #contents',
        'ytd-browse[page-subtype="explore"] #contents',
        'ytm-pivot-bar-item-renderer:has(.pivot-explore)',
        '[data-preelock-hidden="explore"]',
      );
    }

    if (c.navigation.hideSubscriptions) {
      selectors.push(
        'ytd-guide-subscriptions-section-renderer',
        'ytd-guide-collapsible-section-entry-renderer:has(a[href="/feed/subscriptions"])',
        'ytd-guide-collapsible-section-entry-renderer:has(a[title="Subscriptions"])',
        'ytd-guide-entry-renderer:has(a[href="/feed/subscriptions"])',
        'ytd-mini-guide-entry-renderer:has(a[href="/feed/subscriptions"])',
        'ytd-guide-entry-renderer:has(a[title="Subscriptions"])',
        'ytd-mini-guide-entry-renderer[aria-label="Subscriptions"]',
        'ytd-guide-entry-renderer:has(a[href="/feed/channels"])',
        'ytd-guide-entry-renderer[line-end-style="dot"]',
        'ytd-guide-section-renderer:has(a[href="/feed/subscriptions"]):not(:has(a[href="/"])):not(:has(a[href="/feed/you"])):not(:has(a[href="/playlist?list=WL"]))',
        'ytd-guide-section-renderer:has(a[title="Subscriptions"]):not(:has(a[href="/"])):not(:has(a[href="/feed/you"])):not(:has(a[href="/playlist?list=WL"]))',
        'ytd-guide-section-renderer:has(a[href="/feed/channels"]):not(:has(a[href="/"])):not(:has(a[href="/feed/you"])):not(:has(a[href="/playlist?list=WL"]))',
        'ytd-guide-section-renderer:has(a[href^="/@"]):not(:has(a[href="/"])):not(:has(a[href="/feed/you"])):not(:has(a[href="/feed/history"])):not(:has(a[href="/playlist?list=WL"])):not(:has(a[href="/playlist?list=LL"]))',
        'ytd-guide-section-renderer:has(ytd-guide-entry-renderer[line-end-style="dot"]):not(:has(a[href="/"])):not(:has(a[href="/feed/you"])):not(:has(a[href="/playlist?list=WL"]))',
        'ytm-pivot-bar-item-renderer:has(.pivot-subs)',
        'ytm-pivot-bar-item-renderer:has(a[href="/feed/subscriptions"])',
        '[data-preelock-hidden="subscriptions"]',
      );
    }

    // B. Video Recommendations
    if (c.recommendations.hideVideoSidebar) {
      selectors.push(
        'ytd-watch-flexy #secondary',
        'ytd-watch-flexy #secondary-inner',
        'ytd-watch-grid #secondary',
      );
    }

    if (c.recommendations.hideRecommended) {
      selectors.push(
        'ytd-watch-next-secondary-results-renderer',
        'ytd-watch-flexy #related',
        'ytm-item-section-renderer[section-identifier="related-items"]',
        'ytm-watch-next-secondary-results-renderer',
      );
    }

    if (c.recommendations.hideLiveChat) {
      selectors.push(
        'ytd-live-chat-frame',
        '#chat',
        '#chat-container',
        'ytd-live-chat-renderer',
        'ytd-engagement-panel-section-list-renderer[target-id="engagement-panel-live-chat"]',
        '#panels-full-bleed-container:has(ytd-live-chat-frame)',
        'ytm-live-chat-entry-point-renderer',
      );
    }

    if (c.recommendations.hidePlaylist) {
      selectors.push(
        'ytd-watch-flexy ytd-playlist-panel-renderer',
        'ytd-watch-flexy #playlist',
        'ytd-watch-grid ytd-playlist-panel-renderer',
        'ytm-playlist-panel-renderer',
      );
    }

    if (c.recommendations.hideFundraiser) {
      selectors.push(
        'ytd-donation-shelf-renderer',
        '#donation-shelf',
        'ytd-fundraiser-card-renderer',
        'ytd-engagement-panel-section-list-renderer[target-id="engagement-panel-donation"]',
      );
    }

    if (c.recommendations.hideEndScreenFeed) {
      selectors.push(
        'ytd-watch-flexy .ytp-endscreen-content',
        '.html5-endscreen',
        '.ytp-videowall-still',
        '.ytp-endscreen-previous',
        '.ytp-endscreen-next',
        '.ytp-autonav-endscreen-upnext-container',
      );
    }

    if (c.recommendations.hideEndScreenCards) {
      selectors.push(
        '.ytp-ce-element',
        '.ytp-ce-covering-overlay',
        '.ytp-ce-element-shadow',
        '.ytp-ce-covering-image',
        '.ytp-ce-expanding-image',
        '.ytp-ce-video',
        '.ytp-ce-playlist',
        '.ytp-ce-channel',
        '.ytp-ce-website',
      );
    }

    if (c.recommendations.hideShorts) {
      selectors.push(
        'ytd-guide-entry-renderer:has(a[href^="/shorts"])',
        'ytd-mini-guide-entry-renderer:has(a[href^="/shorts"])',
        'ytd-mini-guide-entry-renderer[aria-label="Shorts"]',
        'ytd-guide-entry-renderer:has(a[title="Shorts"])',
        'ytm-pivot-bar-item-renderer:has(.pivot-shorts)',
        'ytd-reel-shelf-renderer',
        'ytd-rich-shelf-renderer[is-shorts]',
        'ytd-rich-section-renderer:has(ytd-rich-shelf-renderer[is-shorts])',
        'ytd-rich-section-renderer:has(ytm-shorts-lockup-view-model)',
        'ytd-rich-section-renderer:has(ytm-shorts-lockup-view-model-v2)',
        'grid-shelf-view-model:has(a[href^="/shorts"])',
        'ytm-shorts-lockup-view-model',
        'ytm-shorts-lockup-view-model-v2',
        'ytm-reel-shelf-renderer',
        'ytd-rich-item-renderer:has(a[href^="/shorts/"])',
        'ytd-video-renderer:has(a[href^="/shorts/"])',
        'ytd-grid-video-renderer:has(a[href^="/shorts/"])',
        'ytd-compact-video-renderer:has(a[href^="/shorts/"])',
        'yt-lockup-view-model:has(a[href^="/shorts/"])',
        'ytd-playlist-video-renderer:has(a[href^="/shorts/"])',
        'ytd-playlist-panel-video-renderer:has(a[href^="/shorts/"])',
        'ytm-video-with-context-renderer:has(a[href^="/shorts/"])',
        'yt-tab-shape[tab-title="Shorts"]',
        '[data-preelock-hidden="shorts"]',
        'html[data-preelock-route="shorts"] ytd-shorts',
        'html[data-preelock-route="shorts"] #shorts-container',
      );
    }

    if (c.recommendations.hideMixes) {
      selectors.push(
        'ytd-radio-renderer',
        'ytd-compact-radio-renderer',
        'ytd-rich-item-renderer:has(a[href*="&list=RD"])',
        'ytd-rich-item-renderer:has(a[href*="?list=RD"])',
        'ytd-compact-video-renderer:has(a[href*="&list=RD"])',
        'ytd-compact-video-renderer:has(a[href*="?list=RD"])',
        'ytd-video-renderer:has(a[href*="&list=RD"])',
        'ytd-video-renderer:has(a[href*="?list=RD"])',
        'ytd-grid-video-renderer:has(a[href*="&list=RD"])',
        'ytd-grid-video-renderer:has(a[href*="?list=RD"])',
        'yt-lockup-view-model:has(a[href*="&list=RD"])',
        'yt-lockup-view-model:has(a[href*="?list=RD"])',
        'ytd-playlist-renderer:has(a[href*="&list=RD"])',
        'ytd-playlist-renderer:has(a[href*="?list=RD"])',
        'ytd-compact-playlist-renderer:has(a[href*="&list=RD"])',
        'ytd-compact-playlist-renderer:has(a[href*="?list=RD"])',
        '[data-preelock-hidden="mix"]',
      );
    }

    // C. Video Player
    if (c.player.hideVideoInfo) {
      selectors.push(
        'ytd-watch-flexy #above-the-fold',
        'ytd-watch-flexy ytd-watch-metadata',
        'ytd-watch-flexy #info-contents',
        'ytd-watch-flexy #meta-contents',
        'ytm-slim-video-metadata-section-renderer',
      );
    }

    if (c.player.hideDescription) {
      selectors.push(
        'ytd-watch-flexy #description',
        'ytd-watch-flexy #description-inner',
        'ytd-watch-flexy ytd-watch-metadata #bottom-row',
        'ytd-watch-flexy ytd-text-inline-expander',
        'ytd-watch-flexy #structured-description',
        'ytd-engagement-panel-section-list-renderer[target-id="engagement-panel-structured-description"]',
      );
    }

    if (c.player.hideMetadata) {
      selectors.push(
        'ytd-watch-flexy ytd-watch-metadata #info-container',
        'ytd-watch-flexy ytd-watch-info-text',
        'ytd-watch-flexy #info-text',
        'ytd-watch-flexy #super-title',
        'ytd-watch-flexy #view-count',
        'ytd-watch-flexy #date-text',
        'ytd-watch-flexy ytd-metadata-row-container-renderer',
      );
    }

    if (c.player.hideActionButtons) {
      selectors.push(
        'ytd-watch-flexy ytd-watch-metadata #actions',
        'ytd-watch-flexy ytd-watch-metadata #actions-inner',
        'ytd-watch-flexy #top-level-buttons-computed',
        'ytd-watch-flexy #menu-container',
        'ytm-slim-video-action-bar-renderer',
      );
    }

    if (c.player.hideChannelInfo) {
      selectors.push(
        'ytd-watch-flexy ytd-watch-metadata #owner',
        'ytd-watch-flexy ytd-video-owner-renderer',
        'ytd-watch-flexy #subscribe-button',
        'ytd-watch-flexy #sponsor-button',
      );
    }

    if (c.player.disableAutoplay) {
      selectors.push('.ytp-autonav-endscreen-upnext-container');
    }

    if (c.player.disableAnnotations) {
      selectors.push(
        '.ytp-cards-teaser',
        '.ytp-cards-button',
        '.iv-drawer',
        '.ytp-iv-video-content',
        '.ytp-suggested-action',
        '.ytp-suggested-action-badge',
        '.annotation',
        '.ytp-pause-overlay-container',
        '.ytp-pause-overlay',
      );
    }

    // D. Social and Community
    if (c.social.hideComments) {
      selectors.push(
        'ytd-comments',
        'ytd-comments#comments',
        '#comments',
        '#comment-teaser',
        'ytd-engagement-panel-section-list-renderer[target-id="engagement-panel-comments-section"]',
        'ytm-comments-entry-point-header-renderer',
        'ytm-comment-section-renderer',
      );
    }

    if (c.social.hideCommentTeaser) {
      selectors.push(
        '#comment-teaser',
        'ytd-comments-entry-point-header-renderer',
        'ytm-comments-entry-point-header-renderer',
      );
    }

    if (c.social.hideCommentReplies) {
      selectors.push(
        'ytd-comment-replies-renderer',
        '#replies',
        'ytm-comment-replies-renderer',
      );
    }

    if (c.social.hideNotifications) {
      selectors.push(
        'ytd-notification-topbar-button-renderer',
        '#masthead ytd-notification-topbar-button-renderer',
        'ytd-masthead button[aria-label*="Notifications"]',
        'ytd-masthead #notification-count',
        'ytm-notification-topbar-button',
      );
    }

    // E. Other YouTube Elements
    if (c.other.hideMerchTicketsOffers) {
      selectors.push(
        'ytd-merch-shelf-renderer',
        '#merch-shelf',
        'ytd-ticket-shelf-renderer',
        'ytd-offer-module-renderer',
        'ytd-shopping-timely-shelf-renderer',
        'ytd-product-shelf-renderer',
        'ytd-product-details-renderer',
        'ytd-companion-slot-renderer',
        'ytd-promoted-sparkles-web-renderer',
        'ytd-action-companion-ad-renderer',
        'ytd-engagement-panel-section-list-renderer[target-id="engagement-panel-shopping"]',
        'ytd-engagement-panel-section-list-renderer[target-id*="product"]',
        '.ytp-featured-product-card',
        'ytd-watch-flexy ytd-metadata-row-container-renderer:has(ytd-merch-shelf-renderer)',
        '[data-preelock-hidden="merch"]',
      );
    }

    if (c.other.hideTopHeader) {
      selectors.push(
        '#masthead-container',
        'ytd-masthead#masthead',
        'ytm-header-bar',
      );
    }

    // Deduplicate while preserving order.
    var unique = [];
    var seen = {};
    for (var i = 0; i < selectors.length; i++) {
      if (!seen[selectors[i]]) {
        seen[selectors[i]] = true;
        unique.push(selectors[i]);
      }
    }
    return unique;
  }

  function controlsCss(rawControls) {
    var c = normalizeYoutubeControls(rawControls);
    var selectors = controlsHiddenSelectors(c);
    var rules = [];
    for (var i = 0; i < selectors.length; i++) {
      rules.push(selectors[i] + '{display:none!important}');
    }
    if (c.other.hideTopHeader) {
      rules.push(
        'ytd-app{--ytd-masthead-height:0px!important;--ytd-toolbar-height:0px!important}',
        'ytd-app #page-manager.ytd-app{margin-top:0!important}',
        'ytd-app tp-yt-app-drawer#guide #guide-spacer{margin-top:0!important}',
        'ytd-mini-guide-renderer{top:0!important}',
      );
    }
    if (c.recommendations.hideShorts) {
      rules.push(
        '#preelock-shorts-cover{position:fixed;inset:0;z-index:2147482000;' +
          'display:flex;align-items:center;justify-content:center;padding:2rem;' +
          'background:#141715;color:#e8ece8;text-align:center;' +
          'font:500 15px/1.6 system-ui,-apple-system,sans-serif}',
        '#preelock-shorts-cover .preelock-shorts-card{max-width:380px;' +
          'padding:24px;border-radius:12px;background:#1b201c;' +
          'border:1px solid #303a31}',
        '#preelock-shorts-cover h2{margin:0 0 8px;font-size:18px;color:#e8ece8}',
        '#preelock-shorts-cover p{margin:0 0 16px;font-size:13px;color:#b4beb4}',
        '#preelock-shorts-cover button{background:#c3ed9b;color:#172213;' +
          'border:0;border-radius:8px;padding:9px 16px;font:600 13px/1 system-ui;cursor:pointer}',
      );
    }
    return rules.join('\n');
  }

  var api = {
    STORAGE_KEY: STORAGE_KEY,
    SCHEMA_VERSION: SCHEMA_VERSION,
    CONTROLS_STYLE_ID: CONTROLS_STYLE_ID,
    DEFAULT_YOUTUBE_CONTROLS: DEFAULT_YOUTUBE_CONTROLS,
    YOUTUBE_CONTROL_GROUPS: YOUTUBE_CONTROL_GROUPS,
    normalizeYoutubeControls: normalizeYoutubeControls,
    getControlValue: getControlValue,
    setControlValue: setControlValue,
    hasActiveYoutubeControls: hasActiveYoutubeControls,
    countActiveControls: countActiveControls,
    isMainYouTubeHost: isMainYouTubeHost,
    isShortsPath: isShortsPath,
    youtubeRouteKind: youtubeRouteKind,
    videoIdFromVideoUrl: videoIdFromVideoUrl,
    controlsHiddenSelectors: controlsHiddenSelectors,
    controlsCss: controlsCss,
  };

  if (root && typeof root === 'object') {
    root.__preelockYoutubeControls = api;
  }
  if (typeof module === 'object' && module && module.exports) {
    module.exports = api;
  }
})(typeof globalThis !== 'undefined' ? globalThis : this);
