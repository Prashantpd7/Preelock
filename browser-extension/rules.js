// Website-rule semantics. This mirrors Rust focus.rs rule_matches()
// exactly (same rule forms, same match decisions) — keep the two in
// sync. A rule is one stored line from the wizard's websites textarea:
// - example.com: matches the host itself and any subdomain.
// - *.example.com: matches any subdomain, but NOT the bare domain.
// - example.com/path: host rule as above, plus the URL path must equal
//   the rule path or start with it followed by / (youtube.com/shorts
//   matches /shorts and /shorts/abc, not /shortsabc and not /).
// - search:<keyword>: never matches a host. It blocks search-engine
//   result pages (Google, Bing, DuckDuckGo, YouTube search) whose
//   query contains the keyword (case-insensitive substring). Keywords
//   travel in policy.keywords, never in policy.domains, because the
//   desktop app strips them out when it builds the policy.
// Enforcement split, stated honestly:
// - declarativeNetRequest (compileRules) enforces host, wildcard and
//   path rules for every request, the adult overlay
//   (policy.extra_blocked) as block rules that outrank allow rules,
//   and per-block exceptions (policy.exceptions) as allow rules at
//   priority 2 in blocklist mode: above block rules (priority 1),
//   below the adult overlay (priority 3). Local files (file://) are
//   never blocked: DNR rules only match http(s)/ws(s) patterns, and
//   urlBlockedByPolicy returns false for non-http(s) protocols.
// - Top-level navigation is additionally checked by urlBlockedByPolicy
//   (worker.js tab replacement), which is the only place search-query
//   blocking can happen: DNR cannot match a query parameter.
// - YouTube channel rules (step 3) are enforced in three places:
//   channel pages and study-mode surface pages by DNR rules compiled
//   here, watch pages by the youtube.js content script (their URLs
//   carry no channel), and the Watch Later playlist route by a DNR
//   allow rule in both website-blocking modes. The full decision,
//   precedence and fail behavior are documented above
//   urlBlockedByPolicy.
// - The focus timer never sees URLs, only hostnames, so path and
//   keyword relevance are handled as documented in focus.rs.
function escapeRegExp(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
// Parse one stored rule. Returns null for anything malformed. Policies
// come from the validated Rust backend, so null here is defensive.
export function parseRule(raw) {
  if (typeof raw !== 'string') return null;
  const s = raw.trim().toLowerCase();
  if (!s) return null;
  if (s.startsWith('search:')) {
    const keyword = s.slice(7).trim().replace(/\s+/g, ' ');
    if (keyword.length < 2) return null;
    return { kind: 'keyword', keyword };
  }
  const slash = s.indexOf('/');
  const hostPart = slash === -1 ? s : s.slice(0, slash);
  let path = slash === -1 ? '' : '/' + s.slice(slash + 1).replace(/\/$/, '');
  if (path === '/') path = '';
  const wildcard = hostPart.startsWith('*.');
  const host = wildcard ? hostPart.slice(2) : hostPart;
  if (
    !host ||
    !/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)+$/.test(
      host,
    ) ||
    host.endsWith('.localhost') ||
    host.endsWith('.local') ||
    /^(\d+\.)+\d+$/.test(host) ||
    (path && /[?#\s]/.test(path))
  )
    return null;
  return { kind: wildcard ? 'wildcard' : 'host', host, path, wildcard };
}
// The shared matcher: rule (string or parsed) against a hostname and a
// URL path (pathname, without query or fragment). Keyword rules never
// match here: they need a search query, not a host.
export function ruleMatches(rule, hostname, pathname = '/') {
  const parsed = typeof rule === 'string' ? parseRule(rule) : rule;
  if (!parsed || parsed.kind === 'keyword') return false;
  const host = String(hostname || '')
    .toLowerCase()
    .replace(/\.$/, '');
  const hostOk = parsed.wildcard
    ? host.endsWith('.' + parsed.host)
    : host === parsed.host || host.endsWith('.' + parsed.host);
  if (!hostOk) return false;
  if (!parsed.path) return true;
  const path = (pathname || '/').toLowerCase();
  return path === parsed.path || path.startsWith(parsed.path + '/');
}
// DNR regex for a host/wildcard/path rule. The host part allows any
// subdomain for plain rules and requires at least one for wildcards,
// and the end anchor stops evil.com lookalikes (notexample.com,
// example.com.evil.com) from matching.
function urlRegex(parsed) {
  const hostPattern = parsed.wildcard
    ? `(?:[a-z0-9-]+\\.)+${escapeRegExp(parsed.host)}`
    : `(?:[a-z0-9-]+\\.)*${escapeRegExp(parsed.host)}`;
  let pattern = `^https?://${hostPattern}(?::\\d+)?`;
  if (parsed.path) pattern += `${escapeRegExp(parsed.path)}(?:[/?#].*)?$`;
  else pattern += `(?:[/?#].*)?$`;
  return pattern;
}
// DNR regex pieces for YouTube channel rules. Handles and legacy
// names match case-insensitively via per-letter classes (DNR regexes
// have no case-insensitive flag to rely on); UC IDs match exactly,
// since they are case-sensitive.
function caseInsensitivePattern(s) {
  return [...s]
    .map((ch) =>
      /[a-zA-Z]/.test(ch)
        ? `[${ch.toLowerCase()}${ch.toUpperCase()}]`
        : escapeRegExp(ch),
    )
    .join('');
}
const YOUTUBE_HOST_PATTERN = '(?:[a-z0-9-]+\\.)*youtube\\.com';
function youtubeChannelRegex(key) {
  if (key.startsWith('@'))
    return `^https?://${YOUTUBE_HOST_PATTERN}/${caseInsensitivePattern(key)}(?:[/?#].*)?$`;
  if (key.startsWith('UC'))
    return `^https?://${YOUTUBE_HOST_PATTERN}/channel/${escapeRegExp(key)}(?:[/?#].*)?$`;
  const [kind, name] = key.split(':');
  return `^https?://${YOUTUBE_HOST_PATTERN}/${kind}/${caseInsensitivePattern(name)}(?:[/?#].*)?$`;
}
// Any channel page (study-mode catch-all): /@..., /channel/...,
// /c/..., /user/... — but not watch, search, playlist or surface
// pages, which start with other segments.
const YOUTUBE_CHANNEL_CATCHALL = `^https?://${YOUTUBE_HOST_PATTERN}/(?:@[^/?#]+|channel/[^/?#]+|c/[^/?#]+|user/[^/?#]+)(?:[/?#].*)?$`;
// Study-mode surface pages: homepage, Shorts, feeds, results.
const YOUTUBE_SURFACE_PATTERNS = [
  `^https?://${YOUTUBE_HOST_PATTERN}/?(?:[?#].*)?$`,
  `^https?://${YOUTUBE_HOST_PATTERN}/shorts(?:[/?#].*)?$`,
  `^https?://${YOUTUBE_HOST_PATTERN}/feed(?:[/?#].*)?$`,
  `^https?://${YOUTUBE_HOST_PATTERN}/results(?:[?#].*)?$`,
];
const YOUTUBE_ENTERTAINMENT_SURFACE_PATTERNS = [
  `^https?://${YOUTUBE_HOST_PATTERN}/shorts(?:[/?#].*)?$`,
  `^https?://${YOUTUBE_HOST_PATTERN}/gaming(?:[/?#].*)?$`,
  `^https?://${YOUTUBE_HOST_PATTERN}/feed/(?:trending|explore)(?:[/?#].*)?$`,
];
// The Watch Later playlist route, and nothing else: the exact
// `/playlist` path on the main YouTube hosts with `list=WL` as a whole
// query parameter (not list=WLxyz, not a `?xlist=WL` lookalike). Watch
// pages and every other playlist never match. Navigation only: page
// assets from a domain-blocked youtube.com stay blocked (see the
// asset limit noted above).
const YOUTUBE_WATCH_LATER_PATTERN = `^https?://(?:www\\.|m\\.)?youtube\\.com(?::\\d+)?/playlist/?\\?(?:[^#]*&)?list=WL(?:[&#].*)?$`;
// The Watch Later video route: /watch with list=WL and a video id. It
// opens at document level so the content script can verify that the
// video is really in the user's Watch Later playlist; the URL alone
// never permits playback (youtubeWatchDecision does).
const YOUTUBE_WATCH_LATER_VIDEO_PATTERN = `^https?://(?:www\\.|m\\.)?youtube\\.com(?::\\d+)?/watch/?\\?(?:[^#]*&)?list=WL(?:[&#].*)?$`;
const SEARCH_ENGINES = [
  { host: 'google.com', param: 'q', pathPrefix: '/search' },
  { host: 'bing.com', param: 'q', pathPrefix: '/search' },
  { host: 'duckduckgo.com', param: 'q', path: '/' },
  { host: 'youtube.com', param: 'search_query', pathPrefix: '/results' },
];
// The search query of a search-engine result page, or null. Only the
// query string is read; it is never stored or sent anywhere.
export function searchPageQuery(urlString) {
  let url;
  try {
    url = new URL(urlString || '');
  } catch {
    return null;
  }
  if (!/^https?:$/.test(url.protocol)) return null;
  const hostname = url.hostname.toLowerCase();
  for (const engine of SEARCH_ENGINES) {
    if (!(hostname === engine.host || hostname.endsWith('.' + engine.host)))
      continue;
    if (engine.pathPrefix && !url.pathname.startsWith(engine.pathPrefix))
      continue;
    if (engine.path && url.pathname !== engine.path) continue;
    const query = url.searchParams.get(engine.param);
    if (query) return { query };
  }
  return null;
}
export function searchQueryBlocked(keywords, query) {
  const q = String(query || '').toLowerCase();
  return (keywords || []).some((k) => k && q.includes(String(k).toLowerCase()));
}
// ---- YouTube channel rules (step 3). Mirrors Rust focus.rs
// (`normalize_youtube_channel`, `channel_key_from_path`) and
// src/services/domains.ts (`cleanYouTubeChannel`) exactly — keep the
// three in sync. Stored keys: `@handle` (lowercase), a bare UC
// channel ID (case preserved), or `c:name` / `user:name`
// (lowercase).
//
// Why watch pages need a content script: a channel page URL contains
// the channel, but a watch page URL (youtube.com/watch?v=...) does
// not — so watch-page enforcement reads the channel from the page
// DOM in youtube.js, while channel pages and study-mode surface
// pages are enforced here by DNR rules.
//
// Two channel modes (policy.youtube_study_mode decides):
// a. Block list: policy.youtube_blocked_channels blocks those
//    channels' pages and videos; the rest of YouTube works.
// b. Study mode: ONLY policy.youtube_allowed_channels play. The
//    extension also blocks the YouTube homepage (/), Shorts
//    (/shorts/*), feed pages (/feed/*) and search results (/results).
//
// Fail behavior: block-list mode is fail-OPEN (only positively
// identified channels are blocked; a video page whose channel cannot
// be read plays). Study mode is fail-CLOSED on video pages, but only
// after a short detection timeout (youtube.js waits for the player
// and a channel for 5 s; a video page with a loaded player and still
// no channel is blocked). Nothing here is permanent: all channel
// rules live in the session policy lease and expire with it.
//
// Precedence with step-2 exceptions: an exception rule matching the
// page URL (host or host+path) disables channel enforcement for that
// page — exceptions override channel rules, since channel rules are
// block rules. The adult overlay and offline mode keep their higher
// precedence. In allowlist mode exceptions are ignored (as in step 2),
// but channel rules still apply on top of the allowlist: a listed
// youtube.com can still have blocked channels, and study mode can
// still restrict it.
//
// Watch-Later exception: when policy.youtube_allow_watch_later is on,
// the user's Watch Later playlist page (playlist?list=WL, see
// isWatchLaterUrl) opens even when youtube.com is not otherwise
// reachable — domain-blocked in blocklist mode, or absent from the
// allowlist in allowlist mode. It is one exact route, never the
// domain: the homepage, Shorts, search, channel pages, other
// playlists and every watch page (list=WL in a video URL included)
// stay blocked exactly as they were. Watch Later overrides domain and
// channel rules for that page only. Adults/offline still win (offline
// blocks everything, the adult overlay blocks before the exception).
// This is extension-enforced only: query parameters are invisible to
// the engine, so the engine still counts youtube.com as not relevant
// on the allowed page when it is not an allowed website. Known limit:
// in blocklist mode with youtube.com domain-blocked, the navigation
// itself is allowed, but YouTube's own page assets (also served from
// youtube.com) remain domain-blocked, so the page may render
// degraded; Watch Later works fully in allowlist mode and wherever
// youtube.com is not domain-blocked (channel-blocked or study-mode
// cases).
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
  return name.length >= 1 && name.length <= 100 && /^[a-z0-9._-]+$/.test(name);
}
function channelKeyFromSegments(segments) {
  const first = segments[0];
  if (!first) return null;
  if (first.startsWith('@')) {
    const body = first.slice(1).toLowerCase();
    return validHandle(body) ? '@' + body : null;
  }
  if (first === 'channel') {
    const id = segments[1];
    return id && validUcId(id) ? id : null;
  }
  if (first === 'c' || first === 'user') {
    const name = segments[1] && segments[1].toLowerCase();
    return name && validLegacyName(name) ? first + ':' + name : null;
  }
  return null;
}
// The channel key for a YouTube URL pathname such as /@handle,
// /@handle/videos, /channel/UC..., /c/name or /user/name, or null
// for any other path (watch, search, playlist, Shorts, feed,
// homepage). Channel tab segments (/videos, /streams) are ignored.
export function channelKeyFromPath(pathname) {
  const segments = String(pathname || '')
    .split('/')
    .filter(Boolean);
  return channelKeyFromSegments(segments);
}
export function isChannelLivePath(pathname) {
  const segments = String(pathname || '')
    .split('/')
    .filter(Boolean);
  if (segments.length < 2) return false;
  if (segments[segments.length - 1].toLowerCase() !== 'live') return false;
  return Boolean(channelKeyFromSegments(segments));
}
// Normalize one channel entry as typed (or one stored key) to its
// stored key, or null when it is not a channel. Accepts @handle, a
// bare UC ID, c:name / user:name, and YouTube URLs (youtube.com,
// www.youtube.com, m.youtube.com, with or without a scheme) whose
// path is a channel page. Video, search, playlist, Shorts, feed and
// embed URLs are rejected: they are not channels.
export function normalizeYouTubeChannel(raw) {
  if (typeof raw !== 'string') return null;
  const s = raw.trim();
  if (!s || /[\u0000-\u001f\u007f]/.test(s) || /\s/.test(s)) return null;
  if (s.startsWith('@')) {
    const body = s.slice(1).toLowerCase();
    return validHandle(body) ? '@' + body : null;
  }
  if (validUcId(s)) return s;
  for (const prefix of ['c:', 'user:']) {
    if (s.startsWith(prefix)) {
      const name = s.slice(prefix.length).toLowerCase();
      return validLegacyName(name) ? prefix + name : null;
    }
  }
  const lower = s.toLowerCase();
  const withoutScheme = lower.replace(/^https?:\/\//, '');
  const withoutQuery = withoutScheme.split(/[?#]/)[0].replace(/\/$/, '');
  const slash = withoutQuery.indexOf('/');
  if (slash === -1) return null;
  const host = withoutQuery.slice(0, slash);
  if (!['youtube.com', 'www.youtube.com', 'm.youtube.com'].includes(host))
    return null;
  // The path keeps the original case for UC IDs: re-derive it from
  // the scheme-stripped original rather than the lowercased copy.
  const schemeLen = lower.startsWith('https://')
    ? 8
    : lower.startsWith('http://')
      ? 7
      : 0;
  const originalPath = s
    .slice(schemeLen)
    .split(/[?#]/)[0]
    .split('/')
    .slice(1)
    .join('/');
  return channelKeyFromPath('/' + originalPath);
}
export function isYouTubeHost(hostname) {
  const host = String(hostname || '').toLowerCase();
  return host === 'youtube.com' || host.endsWith('.youtube.com');
}
// The channel key for one link href (relative or absolute), or null.
// youtube.js collects owner-chip hrefs from the video page DOM and
// takes the first one that parses as a channel.
export function channelKeyFromHref(href) {
  if (typeof href !== 'string' || !href) return null;
  let url;
  try {
    url = new URL(href, 'https://www.youtube.com/');
  } catch {
    return null;
  }
  if (!isYouTubeHost(url.hostname)) return null;
  return channelKeyFromPath(url.pathname);
}
export function extractChannelKey(hrefs) {
  for (const href of hrefs || []) {
    const key = channelKeyFromHref(href);
    if (key) return key;
  }
  return null;
}
// Main YouTube hosts that serve the user's Watch Later playlist.
// YouTube Music and any other subdomain is a different surface and
// never qualifies.
const WATCH_LATER_HOSTS = ['youtube.com', 'www.youtube.com', 'm.youtube.com'];
// The Watch Later playlist route, exactly: `/playlist` (optionally
// with a trailing slash) with the `list` parameter exactly `WL`. A
// longer id (WLxyz), a parameter that merely contains that text
// (`?xlist=WL`), another playlist id, a video URL carrying list=WL
// and any other YouTube path are all rejected.
export function isWatchLaterUrl(urlString) {
  let url;
  try {
    url = new URL(urlString || '');
  } catch {
    return false;
  }
  if (!/^https?:$/.test(url.protocol)) return false;
  if (!WATCH_LATER_HOSTS.includes(url.hostname.toLowerCase())) return false;
  const pathname = url.pathname.toLowerCase();
  if (pathname !== '/playlist' && pathname !== '/playlist/') return false;
  return url.searchParams.get('list') === 'WL';
}
// The Watch Later *video* route: a `/watch` page carrying `list=WL`
// and a video id. This is a route, never a permission — the playlist
// listing itself proves nothing, so youtube.js verifies the video
// against the Watch Later playlist YouTube rendered for the user and
// the worker refuses unverified membership
// (youtubeWatchDecision). Mirrors youtube.js isWatchLaterVideoUrl.
export function isWatchLaterVideoUrl(urlString) {
  let url;
  try {
    url = new URL(urlString || '');
  } catch {
    return false;
  }
  if (!/^https?:$/.test(url.protocol)) return false;
  if (!WATCH_LATER_HOSTS.includes(url.hostname.toLowerCase())) return false;
  const pathname = url.pathname.toLowerCase();
  if (pathname !== '/watch' && pathname !== '/watch/') return false;
  return (
    url.searchParams.get('list') === 'WL' && Boolean(url.searchParams.get('v'))
  );
}
// Whether ordinary YouTube *browsing* is closed by this policy: its
// homepage would be blocked. Watch Later and channel exceptions only
// matter in that case — when YouTube is open anyway, a video that
// happens to carry list=WL is just a normal video.
export function youtubeBrowsingBlocked(policy) {
  if (!policy) return false;
  return urlBlockedByPolicy('https://www.youtube.com/', policy);
}
// Watch Later is the only permitted YouTube content: browsing is
// closed and the toggle is on (never in offline mode or when the
// Watch Later playlist itself is blocked by a higher rule like adult overlay).
const WATCH_LATER_PLAYLIST_URL = 'https://www.youtube.com/playlist?list=WL';
export function watchLaterOnly(policy) {
  return Boolean(
    policy &&
    policy.mode !== 'offline' &&
    policy.youtube_allow_watch_later &&
    youtubeBrowsingBlocked(policy) &&
    !urlBlockedByPolicy(WATCH_LATER_PLAYLIST_URL, policy),
  );
}
// The homepage and general feeds are sent to the Watch Later playlist
// instead of a blocked page when Watch Later is the only permitted
// YouTube content. Anything else (playlist, watch, channel) is allowed
// or blocked on its own, so nothing here can loop: the destination is
// never a redirect source. Only main YouTube hosts qualify (not music/studio).
export function shouldRedirectToWatchLater(urlString, policy) {
  if (!watchLaterOnly(policy)) return false;
  let url;
  try {
    url = new URL(urlString || '');
  } catch {
    return false;
  }
  if (!/^https?:$/.test(url.protocol)) return false;
  if (!WATCH_LATER_HOSTS.includes(url.hostname.toLowerCase())) return false;
  const path = url.pathname.toLowerCase();
  return path === '/' || path === '/feed' || path.startsWith('/feed/');
}
export function watchLaterRedirectUrl(urlString, policy) {
  if (!shouldRedirectToWatchLater(urlString, policy)) return null;
  const url = new URL(urlString);
  const host =
    url.hostname.toLowerCase() === 'm.youtube.com'
      ? 'm.youtube.com'
      : 'www.youtube.com';
  return `https://${host}/playlist?list=WL`;
}
// Study-mode surface pages: homepage (/), Shorts, feeds, results.
// Watch pages and channel pages are NOT surface pages.
export function youtubeSurfaceBlocked(pathname) {
  const path = String(pathname || '/').toLowerCase();
  return (
    path === '/' ||
    path === '/shorts' ||
    path.startsWith('/shorts/') ||
    path === '/feed' ||
    path.startsWith('/feed/') ||
    path === '/results'
  );
}
export function isEntertainmentYouTubeUrl(pathname) {
  const path = String(pathname || '/').toLowerCase();
  return (
    path === '/shorts' ||
    path.startsWith('/shorts/') ||
    path === '/gaming' ||
    path.startsWith('/gaming/') ||
    path === '/feed/trending' ||
    path.startsWith('/feed/trending/') ||
    path === '/feed/explore' ||
    path.startsWith('/feed/explore/')
  );
}
const PURE_ENTERTAINMENT_CATEGORIES = new Set([
  'comedy',
  'pets & animals',
  'gaming',
]);

export const PRODUCTIVE_EDUCATIONAL_PATTERN =
  /(?:\b(?:procrastinat(?:ion|e|ing)?|discipline|self[\s-]?discipline|deep[\s-]?work|focus(?:ed|ing)?|habit(?:s)?|atomic\s+habits|routine(?:s)?|dopamine(?:\s+detox)?|hard\s+(?:things|work)|willpower|mental\s+toughness|monk\s+mode|mindset|stoic(?:ism)?|self[\s-]?improvement|personal\s+development|productivity|productive|time[\s-]?management|pomodoro|digital\s+detox|life\s+without\s+social\s+media|quit(?:ting)?\s+(?:social\s+media|phone|sugar|bad\s+habits|porn|gaming)|social\s+media\s+addiction|phone\s+addiction|stop\s+playing\s+small|burnout|motivation(?:al)?|explain(?:ed|ing|s)?|explanation(?:s)?|how\s+to(?!\s+(?:prank|roast|troll|meme|cheat))\b|why\s+(?:we|you|i|it|is|do|people|they)\b|here['’]?s\s+why|tutorial(?:s)?|guide(?:\s+to)?|beginner['’]?s\s+guide|ultimate\s+guide|course(?:s)?|crash\s+course|masterclass|lecture(?:s)?|lesson(?:s)?|learn(?:ing)?|study(?:ing)?|study\s+skills|study\s+with\s+me|breakdown|analysis|analyzing|deep\s+dive|overview|summary|book\s+summary|case\s+study|documentary|documentaries|video\s+essay(?:s)?|neuroscience|psychology|philosophy|science(?:s)?|scientific|physics|chemistry|biology|astronomy|mathematics|math|algebra|calculus|geometry|statistics|history(?: of)?|historical|geopolitics|geopolitical|economics|finance|investing|stock\s+market|business|career|interview\s+prep|coding|programming|developer(?:s)?|software\s+engine(?:er|ering)|computer\s+science|algorithm(?:s)?|data\s+structures?|system\s+design|web\s+dev(?:elopment)?|machine\s+learning|deep\s+learning|artificial\s+intelligence|\bAI\b|neural\s+network(?:s)?|python|javascript|typescript|rust|golang|c\+\+|java\b|react(?:js|\.js)?|node(?:\.js)?|sql|linux|docker|kubernetes|ted\s+talk(?:s)?|\btedx\b|keynote|discussion|podcast|interview(?:s)?)\b)/i;

export const STRONG_ENTERTAINMENT_PATTERN =
  /(?:\b(?:stand[\s-]?up\s+comed(?:y|ian)|stand[\s-]?up\s+special|stand[\s-]?up\b|comedian(?:s)?|comedy\s+(?:show|scenes?|special|club|night|circus|sketch|skit|clash|king|video)|comedy\b|comedic\b|sitcom(?:s)?|roast(?:s|ed|ing)?\s+show|comedy\s+roast|roasting\s+(?:the\s+)?.*(?:tiktokers|youtubers|streamers|celebrities)|roast\s+of\b|sketch\s+comedy|comedy\s+skit(?:s)?|parod(?:y|ies)|spoof(?:s)?|prank(?:s|ed|ing|ster)?|social\s+experiment|try\s+not\s+to\s+laugh|fail\s+compilation(?:s)?|bloopers?|cringe\s+compilation(?:s)?|memes?|shitpost(?:s|ing)?|dank\s+memes?|funny\s+(?:videos?|clips?|moments?|fails?|animals?|pets?|cats?|dogs?|cats\s+and\s+dogs)|(?:cute|cutest)\s+(?:animals?|pets?|cats?|dogs?|puppies|kittens)|funniest\s+(?:videos?|clips?|moments?|fails?|animals?)|hilarious\s+(?:clips?|moments?|fails?|comedy)|cat\s+videos?|dog\s+videos?|gameplay(?:\s+walkthrough)?|playthrough|let['’]?s\s+play|gaming\s+montage|funny\s+gaming\s+moments|speedrun|mukbang|eating\s+show|food\s+challenge|last\s+to\s+leave|24\s+hours\s+(?:in|at|with)\s+challenge|official\s+(?:movie\s+)?trailer|teaser\s+trailer|movie\s+trailer|film\s+trailer|official\s+teaser|teaser\b|trailer\b|movie\s+clips?|official\s+music\s+video|official\s+lyric\s+video|music\s+video|celebrity\s+gossip|paparazzi|drama\s+alert|tea\s+spill|countdown\s+to\s+.*\b(?:avengers|marvel|disney\+|star\s+wars|dc|batman|superman)\b|daily\s+vlog(?:s)?|family\s+vlog(?:s)?|travel\s+vlog(?:s)?|lifestyle\s+vlog(?:s)?|vacation\s+vlog(?:s)?|\bgrwm\b|get\s+ready\s+with\s+me|day\s+in\s+my\s+life\s+vlog)\b|#(?:comedy|standup|prank|pranks|dailyvlog|minivlog|meme|memes|pets|animals|trynottolaugh|mukbang|gaming)\b)/i;

export function isProductiveText(text) {
  if (typeof text !== 'string' || !text.trim()) return false;
  return PRODUCTIVE_EDUCATIONAL_PATTERN.test(text);
}

export function isEntertainmentText(text) {
  if (typeof text !== 'string' || !text.trim()) return false;
  return STRONG_ENTERTAINMENT_PATTERN.test(text);
}
export function hasEntertainmentMeta(videoMeta) {
  return Boolean(
    videoMeta &&
    (String(videoMeta.category || '').trim() ||
      String(videoMeta.title || '').trim()),
  );
}
export function isEntertainmentVideo(videoMeta, urlString = '') {
  if (urlString) {
    try {
      const url = new URL(urlString);
      if (
        isYouTubeHost(url.hostname) &&
        isEntertainmentYouTubeUrl(url.pathname)
      )
        return true;
    } catch {}
  }
  if (!videoMeta || typeof videoMeta !== 'object') return false;

  const title = String(videoMeta.title || '').trim();
  const category = String(videoMeta.category || '')
    .trim()
    .toLowerCase();
  const author = String(
    videoMeta.author || videoMeta.channelTitle || '',
  ).trim();
  const hashtags = (
    Array.isArray(videoMeta.hashtags) ? videoMeta.hashtags : []
  ).join(' ');
  const keywords = (
    Array.isArray(videoMeta.keywords) ? videoMeta.keywords : []
  ).join(' ');

  const titleHasProductive = title ? isProductiveText(title) : false;
  const titleHasEntertainment = title ? isEntertainmentText(title) : false;

  // 1. If title matches strong entertainment (comedy, standup, pranks, memes, trailers):
  if (titleHasEntertainment) {
    if (!titleHasProductive) return true;
    // Both matched: check if strong comedy/prank signals are primary
    if (
      /\b(?:stand[\s-]?up|comed(?:y|ian)|prank|roast|meme|mukbang|let['’]?s\s+play|gameplay|try\s+not\s+to\s+laugh|parod(?:y|ies)|vlog)\b/i.test(
        title,
      )
    ) {
      return true;
    }
  }

  // 2. If title matches positive discipline, productivity, or educational topics:
  if (titleHasProductive) {
    return false;
  }

  // 3. Category evaluation for unambiguous entertainment categories:
  if (category && PURE_ENTERTAINMENT_CATEGORIES.has(category)) {
    return true;
  }

  // 4. Hashtags evaluation:
  if (hashtags && isEntertainmentText(hashtags)) {
    return true;
  }

  // 5. Author/Channel name with explicit comedy/prank keywords:
  if (
    author &&
    /\b(?:stand[\s-]?up\s+comed(?:y|ian)|pranks?|roast|memes?)\b/i.test(author)
  ) {
    return true;
  }

  // 6. Keywords with explicit comedy/prank/gaming keywords (when title is not productive):
  if (
    keywords &&
    /\b(?:stand[\s-]?up\s+comed(?:y|ian)|comedy\s+(?:special|show)|prank(?:s|ed|ing)?|roast\s+show|try\s+not\s+to\s+laugh|mukbang)\b/i.test(
      keywords,
    )
  ) {
    return true;
  }

  return false;
}
// Channel-page decision (channel pages only; returns false for any
// other path). In study mode a channel plays only when listed;
// otherwise a channel is blocked only when listed as blocked.
export function channelPageBlocked(pathname, policy) {
  const key = channelKeyFromPath(pathname);
  if (!key) return false;
  if (policy?.youtube_study_mode)
    return !(policy.youtube_allowed_channels || []).includes(key);
  return (policy?.youtube_blocked_channels || []).includes(key);
}
// Watch-page decision, given the channel read from the page DOM (or
// null when it could not be read). Block mode is fail-OPEN (an
// unidentified channel plays); study mode is fail-CLOSED only when
// `unidentified` is set, which youtube.js sets only after its
// detection timeout with a loaded player.
export function youtubeWatchBlocked(channelKey, policy, unidentified = false) {
  if (!policy) return false;
  if (policy.youtube_study_mode) {
    if (channelKey)
      return !(policy.youtube_allowed_channels || []).includes(channelKey);
    return Boolean(unidentified);
  }
  return Boolean(
    channelKey && (policy.youtube_blocked_channels || []).includes(channelKey),
  );
}
// Why a channel/surface page is channel-blocked, or null. Watch pages
// are never answered here (their URLs carry no channel); see
// youtubeWatchDecision. Exceptions are NOT checked here: callers that
// need the exception override (urlBlockedByPolicy, the worker's
// youtube_check) check exceptions first.
export function channelBlockReason(urlString, policy) {
  if (!policy) return null;
  let url;
  try {
    url = new URL(urlString || '');
  } catch {
    return null;
  }
  if (!/^https?:$/.test(url.protocol)) return null;
  if (!isYouTubeHost(url.hostname)) return null;
  if (
    !policy.youtube_study_mode &&
    !policy.youtube_block_entertainment &&
    !(policy.youtube_blocked_channels || []).length
  )
    return null;
  if (policy.youtube_study_mode && youtubeSurfaceBlocked(url.pathname))
    return 'study_surface';
  if (
    policy.youtube_block_entertainment &&
    isEntertainmentYouTubeUrl(url.pathname)
  )
    return 'entertainment_surface';
  const key = channelKeyFromPath(url.pathname);
  if (!key) return null;
  if (policy.youtube_study_mode)
    return (policy.youtube_allowed_channels || []).includes(key)
      ? null
      : 'study_channel';
  return (policy.youtube_blocked_channels || []).includes(key)
    ? 'blocked_channel'
    : null;
}
// Full-URL policy decision for top-level navigation (worker.js tab
// replacement). Precedence, identical to Rust focus.rs/engine.rs:
// 1. Offline mode: everything is blocked; exceptions do NOT apply.
// 2. Adult overlay (policy.extra_blocked): blocked; exceptions do
//    NOT override it.
// 3. Exceptions (policy.exceptions, blocklist mode only): a match
//    ALLOWS the URL, overriding blocklist rules AND keyword rules —
//    including keyword blocks on a search engine whose host matches
//    an exception (the exception is checked before the keyword rule).
//    Exceptions also disable channel enforcement for that page.
// 4. Watch Later (the exact playlist?list=WL route, both modes):
//    ALLOWED, overriding domain/allowlist and channel rules. It never
//    covers any other YouTube route, so videos, Shorts, search,
//    channels and other playlists keep whatever decision they had.
// 5. Keyword rules, then block rules: blocklist blocks on rule match;
//    allowlist blocks anything not in the allowed list (exceptions are
//    meaningless there, since the allowed list already defines what is
//    open, so they are ignored in allowlist mode).
// 6. Channel rules: a channel page or study-mode surface page the
//    channel rules block is blocked. Watch pages are NOT decided
//    here — their URLs carry no channel, so the youtube.js content
//    script enforces them (see youtubeWatchDecision).
// Local loopback is always allowed, so the app's own bridge keeps
// working. Local files (file:// URLs) are never blocked: this
// function returns false for any non-http(s) protocol, and DNR
// cannot match file:// URLs anyway (see compileRules) — so no
// exception entry is needed for them. Extension internal pages
// (chrome-extension://) are likewise never blocked, for the same
// reason. Exceptions apply only while a session/block is active;
// they never permanently allow or block anything.
export function urlBlockedByPolicy(urlString, policy) {
  if (!policy) return false;
  let url;
  try {
    url = new URL(urlString || '');
  } catch {
    return false;
  }
  if (!/^https?:$/.test(url.protocol)) return false;
  const hostname = url.hostname.toLowerCase();
  if (['localhost', '127.0.0.1', '[::1]'].includes(hostname)) return false;
  if (policy.mode === 'offline') return true;
  const domains = (policy.domains || []).map(String);
  const extra = (policy.extra_blocked || []).map(String);
  const exceptions = (policy.exceptions || []).map(String);
  if (extra.some((d) => ruleMatches(d, hostname, url.pathname))) return true;
  if (
    policy.mode === 'blocklist' &&
    exceptions.some((d) => ruleMatches(d, hostname, url.pathname))
  )
    return false;
  // Watch Later overrides domain, allowlist and channel rules for its
  // one exact playlist route — but only when the toggle is on.
  // Offline mode and the adult overlay were already decided above, so
  // they still win.
  if (policy.youtube_allow_watch_later && isWatchLaterUrl(urlString))
    return false;
  // The Watch Later video route opens as well, because its URL alone
  // cannot prove membership: youtube.js checks the video against the
  // playlist YouTube rendered and the worker keeps or blocks it.
  if (policy.youtube_allow_watch_later && isWatchLaterVideoUrl(urlString))
    return false;
  const found = searchPageQuery(urlString);
  if (found && searchQueryBlocked(policy.keywords || [], found.query))
    return true;
  const listed = domains.some((d) => ruleMatches(d, hostname, url.pathname));
  if (policy.mode === 'blocklist' && listed) return true;
  if (policy.mode === 'allowlist' && !listed) return true;
  if (policy.mode !== 'blocklist' && policy.mode !== 'allowlist') return false;
  // Channel rules, after domain rules: a channel page the domain
  // rules allow can still be channel-blocked, and a study-mode
  // surface page is blocked even when youtube.com is allowed.
  // Watch pages are not decided here (no channel in their URL).
  if (
    !(policy.youtube_allow_watch_later && isWatchLaterUrl(urlString)) &&
    channelBlockReason(urlString, policy)
  )
    return true;
  return false;
}
// Which rule blocks this URL, for the blocked page's "why" line.
// Mirrors urlBlockedByPolicy's precedence exactly; returns
// { blocked: false } or { blocked: true, mechanism, rule }.
// `rule` is the matched extra_blocked rule string (for per-rule
// reason lookup in policy.rule_causes); session-level mechanisms
// (domain, keyword, allowlist, offline, channel) carry rule: null
// and use the policy's session_cause instead.
export function blockDetail(urlString, policy) {
  if (!policy) return { blocked: false };
  let url;
  try {
    url = new URL(urlString || '');
  } catch {
    return { blocked: false };
  }
  if (!/^https?:$/.test(url.protocol)) return { blocked: false };
  const hostname = url.hostname.toLowerCase();
  if (['localhost', '127.0.0.1', '[::1]'].includes(hostname))
    return { blocked: false };
  if (policy.mode === 'offline')
    return { blocked: true, mechanism: 'offline', rule: null };
  const pathname = url.pathname;
  const extra = (policy.extra_blocked || []).map(String);
  const hitExtra = extra.find((d) => ruleMatches(d, hostname, pathname));
  if (hitExtra) return { blocked: true, mechanism: 'extra', rule: hitExtra };
  const exceptions = (policy.exceptions || []).map(String);
  if (
    policy.mode === 'blocklist' &&
    exceptions.some((d) => ruleMatches(d, hostname, pathname))
  )
    return { blocked: false };
  if (policy.youtube_allow_watch_later && isWatchLaterUrl(urlString))
    return { blocked: false };
  if (policy.youtube_allow_watch_later && isWatchLaterVideoUrl(urlString))
    return { blocked: false };
  const found = searchPageQuery(urlString);
  if (found && searchQueryBlocked(policy.keywords || [], found.query))
    return { blocked: true, mechanism: 'keyword', rule: null };
  const domains = (policy.domains || []).map(String);
  const listed = domains.some((d) => ruleMatches(d, hostname, pathname));
  if (policy.mode === 'blocklist' && listed)
    return { blocked: true, mechanism: 'domain', rule: null };
  if (policy.mode === 'allowlist' && !listed)
    return { blocked: true, mechanism: 'allowlist', rule: null };
  if (policy.mode !== 'blocklist' && policy.mode !== 'allowlist')
    return { blocked: false };
  const channelReason =
    !(policy.youtube_allow_watch_later && isWatchLaterUrl(urlString)) &&
    channelBlockReason(urlString, policy);
  if (channelReason)
    return {
      blocked: true,
      mechanism:
        channelReason === 'entertainment_surface' ? 'entertainment' : 'channel',
      rule: null,
    };
  return { blocked: false };
}
// Watch-page decision for the worker's youtube_check message, given
// the channel youtube.js read from the page DOM (or null). Precedence:
// a blocklist exception allows the page outright; otherwise the domain
// decision applies, then the watch decision. Watch Later is NOT an
// exemption here: it covers the playlist page only, so a video URL
// that merely carries list=WL is decided like any other video.
// `unidentified` is set by youtube.js only after its detection
// timeout with a loaded player, so study mode fails closed only then.
// Decision for one video, given the channel youtube.js read from the
// page (or null) and whether the video is verified to be in the user's
// Watch Later playlist (`true`), positively not verifiable (`false`),
// or still being verified (`null`). Precedence is explicit and shared
// with the rest of the session policy:
// 1. An exception rule for the page allows it outright.
// 2. An explicitly blocked channel always wins — over Watch Later, over
//    study-mode allowlists and over any entry point.
// 3. Study mode: an approved channel plays; a verified Watch Later
//    video is the additional exception; everything else is blocked
//    (unknown channel/identity only after the detection timeout).
// 4. The page's own domain/allowlist decision applies.
// 5. When the Watch Later route is the only reason the page opened, the
//    video must be verified as a Watch Later member; an unverified
//    video is blocked once the script gives up waiting.
// `{ pending: true }` means "keep the neutral cover, ask again" — it is
// never an allow.
// Every identity the content script could read for one video, normalized
// to the stored key forms. Junk and duplicates are dropped; an empty list
// means "identity not known yet", never "allowed".
export function channelIdentityKeys(raw) {
  const out = [];
  for (const entry of raw || []) {
    const key = normalizeYouTubeChannel(String(entry ?? ''));
    if (key && !out.includes(key)) out.push(key);
  }
  return out;
}
// The authoritative video decision. `identities` are all verified
// identities read for the *currently playing* video (player channel id,
// owner handle, metadata), already expanded through the learned
// UC-id/handle alias map. Precedence is explicit:
// 1. A blocklist exception for the page allows it.
// 2. Any explicitly blocked identity blocks it — over Watch Later, over
//    study-mode allowlists, from every entry point.
// 3. Study mode: an approved identity allows it; a verified Watch Later
//    video is the additional exception; anything else is blocked.
// 4. The page's own domain/allowlist decision applies.
// 5. When the Watch Later route is the only reason the page opened, the
//    video must be a verified Watch Later member.
// 6. With channel blocking configured, an unidentified video is never
//    allowed: it stays pending, then is blocked once the content script
//    says the resolution window is over.
// `{ pending: true }` means "keep the player protected, ask again"; it is
// never an allow.
export function youtubeVideoDecision(
  urlString,
  identities,
  policy,
  unidentified = false,
  watchLaterMember = null,
  videoMeta = null,
) {
  if (!policy) return { blocked: false, reason: null };
  let url;
  try {
    url = new URL(urlString || '');
  } catch {
    return { blocked: false, reason: null };
  }
  if (!/^https?:$/.test(url.protocol)) return { blocked: false, reason: null };
  if (!isYouTubeHost(url.hostname)) return { blocked: false, reason: null };
  if (
    policy.mode === 'blocklist' &&
    (policy.exceptions || []).some((d) =>
      ruleMatches(d, url.hostname, url.pathname),
    )
  )
    return { blocked: false, reason: 'exception' };
  const keys = (identities || []).filter(Boolean);
  if (keys.some((key) => (policy.youtube_blocked_channels || []).includes(key)))
    return { blocked: true, reason: 'blocked_channel' };
  const urlChannelReason =
    policy.youtube_allow_watch_later && isWatchLaterUrl(urlString)
      ? null
      : channelBlockReason(urlString, policy);
  if (
    urlChannelReason &&
    !(
      urlChannelReason === 'study_channel' &&
      keys.length &&
      keys.some((key) => (policy.youtube_allowed_channels || []).includes(key))
    )
  )
    return { blocked: true, reason: urlChannelReason };
  if (!policy.youtube_study_mode && urlBlockedByPolicy(urlString, policy))
    return { blocked: true, reason: 'domain' };
  const pathname = url.pathname.toLowerCase();
  const isVideoRoute =
    pathname === '/watch' ||
    pathname === '/watch/' ||
    pathname.startsWith('/shorts/') ||
    pathname.startsWith('/live/') ||
    isChannelLivePath(url.pathname);
  // Never authorize Watch Later (or anything else) before channel
  // identity is verified when explicit channel blocks are configured.
  if (
    isVideoRoute &&
    (policy.youtube_blocked_channels || []).length &&
    !keys.length
  ) {
    if (!unidentified) return { blocked: false, reason: null, pending: true };
    return { blocked: true, reason: 'channel_unidentified' };
  }
  const watchLaterRoute = Boolean(
    policy.youtube_allow_watch_later && isWatchLaterVideoUrl(urlString),
  );
  if (policy.youtube_study_mode) {
    if (
      keys.some((key) => (policy.youtube_allowed_channels || []).includes(key))
    )
      return { blocked: false, reason: null };
    if (watchLaterRoute) {
      if (watchLaterMember === true)
        return { blocked: false, reason: 'watch_later' };
      if (watchLaterMember === null)
        return { blocked: false, reason: null, pending: true };
      return { blocked: true, reason: 'watch_later_unverified' };
    }
    if (!keys.length && !unidentified)
      return { blocked: false, reason: null, pending: true };
    return {
      blocked: true,
      reason: keys.length ? 'study_channel' : 'study_unidentified',
    };
  }
  if (urlBlockedByPolicy(urlString, policy))
    return { blocked: true, reason: 'domain' };
  if (watchLaterRoute && watchLaterOnly(policy)) {
    if (watchLaterMember === false)
      return { blocked: true, reason: 'watch_later_unverified' };
    if (watchLaterMember === null && !unidentified)
      return { blocked: false, reason: null, pending: true };
    if (watchLaterMember !== true)
      return { blocked: true, reason: 'watch_later_unverified' };
    if (policy.youtube_block_entertainment && isVideoRoute) {
      if (isEntertainmentVideo(videoMeta, urlString))
        return { blocked: true, reason: 'entertainment' };
      if (!hasEntertainmentMeta(videoMeta) && !unidentified)
        return { blocked: false, reason: null, pending: true };
    }
    return { blocked: false, reason: 'watch_later' };
  }
  if (policy.youtube_block_entertainment && isVideoRoute) {
    if (isEntertainmentVideo(videoMeta, urlString))
      return { blocked: true, reason: 'entertainment' };
    if (!hasEntertainmentMeta(videoMeta) && !unidentified)
      return { blocked: false, reason: null, pending: true };
  }
  return { blocked: false, reason: null };
}
// Back-compat wrapper for a single identity.
export function youtubeWatchDecision(
  urlString,
  channelKey,
  policy,
  unidentified = false,
  watchLaterMember = null,
) {
  return youtubeVideoDecision(
    urlString,
    channelKey ? [channelKey] : [],
    policy,
    unidentified,
    watchLaterMember,
  );
}
// Kept for callers that only have one identity; see youtubeVideoDecision.
export function legacyYoutubeWatchDecision(
  urlString,
  channelKey,
  policy,
  unidentified = false,
  watchLaterMember = null,
) {
  if (!policy) return { blocked: false, reason: null };
  let url;
  try {
    url = new URL(urlString || '');
  } catch {
    return { blocked: false, reason: null };
  }
  if (!/^https?:$/.test(url.protocol)) return { blocked: false, reason: null };
  if (!isYouTubeHost(url.hostname)) return { blocked: false, reason: null };
  if (
    policy.mode === 'blocklist' &&
    (policy.exceptions || []).some((d) =>
      ruleMatches(d, url.hostname, url.pathname),
    )
  )
    return { blocked: false, reason: 'exception' };
  // Explicit channel block: the highest rule after the adult overlay.
  if (
    channelKey &&
    (policy.youtube_blocked_channels || []).includes(channelKey)
  )
    return { blocked: true, reason: 'blocked_channel' };
  const watchLaterRoute = Boolean(
    policy.youtube_allow_watch_later && isWatchLaterVideoUrl(urlString),
  );
  if (policy.youtube_study_mode) {
    if (
      channelKey &&
      (policy.youtube_allowed_channels || []).includes(channelKey)
    )
      return { blocked: false, reason: null };
    if (watchLaterRoute) {
      if (watchLaterMember === true)
        return { blocked: false, reason: 'watch_later' };
      if (watchLaterMember === null && !unidentified)
        return { blocked: false, reason: null, pending: true };
      return { blocked: true, reason: 'watch_later_unverified' };
    }
    // Still identifying: fail closed only once youtube.js says the
    // detection window is over.
    if (!channelKey && !unidentified)
      return { blocked: false, reason: null, pending: true };
    return {
      blocked: true,
      reason: channelKey ? 'study_channel' : 'study_unidentified',
    };
  }
  // Domain / allowlist rules for the page itself.
  if (urlBlockedByPolicy(urlString, policy))
    return { blocked: true, reason: 'domain' };
  // A video that only opened because of the Watch Later route must
  // prove it is in the playlist. `pending` keeps the neutral cover up
  // while the playlist panel (or a cached id) is still arriving.
  if (watchLaterRoute && watchLaterOnly(policy)) {
    if (watchLaterMember === true)
      return { blocked: false, reason: 'watch_later' };
    if (watchLaterMember === null && !unidentified)
      return { blocked: false, reason: null, pending: true };
    return { blocked: true, reason: 'watch_later_unverified' };
  }
  if (youtubeWatchBlocked(channelKey, policy, unidentified))
    return { blocked: true, reason: 'blocked_channel' };
  return { blocked: false, reason: null };
}
// Pure rule compiler: no inspection of page content or URL history.
export function compileRules(policy) {
  if (!policy) return [];
  const types = [
    'main_frame',
    'sub_frame',
    'stylesheet',
    'script',
    'image',
    'font',
    'object',
    'xmlhttprequest',
    'ping',
    'csp_report',
    'media',
    'websocket',
    'webtransport',
    'webbundle',
    'other',
  ];
  const local = ['localhost', '127.0.0.1', '[::1]'];
  const parsed = (policy.domains || []).map((d) => parseRule(d));
  if (parsed.some((p) => p === null)) throw new Error('Invalid policy domains');
  const hostRules = parsed.filter((p) => p.kind !== 'keyword');
  // Exceptions are host/path rules only: a keyword entry here is
  // invalid (the Rust validator rejects search: exceptions), and a
  // malformed entry is invalid too. They are parsed (and rejected)
  // in every mode, but only applied in blocklist mode below.
  const exceptionRules = (policy.exceptions || []).map((d) => parseRule(d));
  if (exceptionRules.some((p) => !p || p.kind === 'keyword'))
    throw new Error('Invalid policy exceptions');
  const extra = (policy.extra_blocked || [])
    .map((d) => parseRule(d))
    .filter(Boolean);
  // Channel lists come normalized from the Rust backend; a malformed
  // entry is invalid, like a malformed domain.
  const blockedChannels = (policy.youtube_blocked_channels || []).map((d) =>
    normalizeYouTubeChannel(d),
  );
  if (blockedChannels.some((k) => k === null))
    throw new Error('Invalid policy youtube_blocked_channels');
  const allowedChannels = (policy.youtube_allowed_channels || []).map((d) =>
    normalizeYouTubeChannel(d),
  );
  if (allowedChannels.some((k) => k === null))
    throw new Error('Invalid policy youtube_allowed_channels');
  const studyMode = Boolean(policy.youtube_study_mode);
  const blockEntertainment = Boolean(policy.youtube_block_entertainment);
  const watchLater = Boolean(policy.youtube_allow_watch_later);
  const rules = [];
  let nextId = 0;
  const add = (action, condition, priority) => {
    rules.push({
      id: ++nextId,
      priority,
      action: { type: action },
      condition,
    });
  };
  // Plain host rules use requestDomains (the browser's own subdomain
  // matching); wildcard and path rules need a regex, since
  // requestDomains cannot express either.
  const domainCondition = (p, resourceTypes) =>
    p.kind === 'host' && !p.path
      ? { requestDomains: [p.host], resourceTypes }
      : { regexFilter: urlRegex(p), resourceTypes };
  if (policy.mode === 'blocklist') {
    // DNR priority decides ties: the adult overlay blocks at
    // priority 3, exception allows at priority 2, and ordinary block
    // rules at priority 1 — so adult > exception > block, matching
    // urlBlockedByPolicy. In allowlist/offline mode exceptions are
    // ignored (see above), so no allow rules are added for them.
    const seen = new Set();
    for (const p of hostRules) {
      const key = `${p.kind}:${p.host}:${p.path}`;
      if (seen.has(key)) continue;
      seen.add(key);
      add('block', domainCondition(p, types), 1);
    }
    const seenExceptions = new Set();
    for (const p of exceptionRules) {
      const key = `${p.kind}:${p.host}:${p.path}`;
      if (seenExceptions.has(key)) continue;
      seenExceptions.add(key);
      add('allow', domainCondition(p, types), 2);
    }
    // The adult overlay is deduped only against itself, never against
    // the block rules: if an adult domain is also a block rule, its
    // priority-3 block must still exist, or a priority-2 exception
    // allow would outrank the priority-1 block and wrongly allow it.
    const seenExtra = new Set();
    for (const p of extra) {
      const key = `${p.kind}:${p.host}:${p.path}`;
      if (seenExtra.has(key)) continue;
      seenExtra.add(key);
      add('block', domainCondition(p, types), 3);
    }
    // YouTube channel rules (step 3). They reuse the existing tiers:
    // channel blocks at 1 (like domain blocks), channel allows at 2
    // (like exception allows), so the adult overlay (3) still wins
    // and an excepted channel page is still allowed by its exception
    // rule. Watch pages get NO DNR rule here — their URLs carry no
    // channel, so the youtube.js content script enforces them.
    // The Watch Later playlist route is allowed at 2 so it outranks
    // a domain block at 1; its page assets are a separate matter (see
    // the Watch-Later note above).
    if (watchLater) {
      add(
        'allow',
        {
          regexFilter: YOUTUBE_WATCH_LATER_PATTERN,
          resourceTypes: ['main_frame', 'sub_frame'],
        },
        2,
      );
      // The video route opens too. Its URL is not a permission: the
      // content script verifies membership and replaces the tab when
      // the video is not in the playlist.
      add(
        'allow',
        {
          regexFilter: YOUTUBE_WATCH_LATER_VIDEO_PATTERN,
          resourceTypes: ['main_frame', 'sub_frame'],
        },
        2,
      );
    }
    // A permitted YouTube page must still load its own scripts, styles
    // and images when youtube.com is a blocked domain. These rules cover
    // subresources only — documents stay subject to the block and
    // channel rules — so no browsing route opens up, and no other
    // domain is touched.
    if (watchLater || studyMode || blockedChannels.length > 0) {
      add(
        'allow',
        {
          regexFilter: `^https?://${YOUTUBE_HOST_PATTERN}(?::\\d+)?/`,
          resourceTypes: types.filter(
            (t) => t !== 'main_frame' && t !== 'sub_frame',
          ),
        },
        2,
      );
    }
    if (studyMode) {
      for (const pattern of YOUTUBE_SURFACE_PATTERNS)
        add('block', { regexFilter: pattern, resourceTypes: types }, 1);
      add(
        'block',
        { regexFilter: YOUTUBE_CHANNEL_CATCHALL, resourceTypes: types },
        1,
      );
      const seenAllowed = new Set();
      for (const key of allowedChannels) {
        if (seenAllowed.has(key)) continue;
        seenAllowed.add(key);
        add(
          'allow',
          { regexFilter: youtubeChannelRegex(key), resourceTypes: types },
          2,
        );
      }
    } else {
      const seenBlocked = new Set();
      for (const key of blockedChannels) {
        if (seenBlocked.has(key)) continue;
        seenBlocked.add(key);
        add(
          'block',
          { regexFilter: youtubeChannelRegex(key), resourceTypes: types },
          1,
        );
      }
    }
    if (blockEntertainment) {
      for (const pattern of YOUTUBE_ENTERTAINMENT_SURFACE_PATTERNS) {
        if (studyMode && !pattern.includes('/gaming')) continue;
        add('block', { regexFilter: pattern, resourceTypes: types }, 1);
      }
    }
    return rules;
  }
  if (!['allowlist', 'offline'].includes(policy.mode))
    throw new Error('Unknown network policy');
  // Navigation types only. An allowed page must load its own styles,
  // scripts, images and fonts (often served from other domains, e.g.
  // github.githubassets.com) or it renders broken. Subresource requests
  // are therefore never filtered in allowlist mode; only opening a page
  // (top-level or iframe) outside the allowed list is blocked.
  const navigation = ['main_frame', 'sub_frame'];
  // Allowlist is implemented as explicit ALLOW rules (higher priority)
  // plus a catch-all BLOCK rule, instead of a single rule with
  // excludedRequestDomains. The adult overlay blocks at priority 3,
  // above the allow rules, so the toggle wins over an explicit listing.
  const allowed =
    policy.mode === 'allowlist' ? [...local, ...hostRules] : [...local];
  for (const entry of allowed) {
    if (typeof entry === 'string') {
      add(
        'allow',
        {
          requestDomains: [entry],
          resourceTypes: policy.mode === 'offline' ? types : navigation,
        },
        2,
      );
    } else {
      add('allow', domainCondition(entry, navigation), 2);
    }
  }
  if (policy.mode === 'allowlist') {
    for (const p of extra) add('block', domainCondition(p, navigation), 3);
  }
  rules.push({
    id: ++nextId,
    priority: 1,
    action: { type: 'block' },
    condition: {
      regexFilter: '^(https?|wss?)://',
      // Offline blocks every request type; allowlist blocks navigation
      // only so allowed pages can load their styles, scripts and images.
      resourceTypes: policy.mode === 'offline' ? types : navigation,
    },
  });
  if (policy.mode === 'allowlist') {
    // The Watch Later exception is one allowed route at priority 2:
    // above the catch-all block (1) and below the adult overlay (3),
    // so the playlist page opens even when youtube.com is not in the
    // session's allowlist. It never allows the domain — every other
    // YouTube route still falls through to the catch-all.
    if (watchLater) {
      add(
        'allow',
        {
          regexFilter: YOUTUBE_WATCH_LATER_PATTERN,
          resourceTypes: navigation,
        },
        2,
      );
      // A Watch Later video opens at document level and is verified (or
      // replaced) by the content script. Subresources are already
      // unfiltered in allowlist mode, so the page renders normally.
      add(
        'allow',
        {
          regexFilter: YOUTUBE_WATCH_LATER_VIDEO_PATTERN,
          resourceTypes: navigation,
        },
        2,
      );
    }
    // Channel rules sit above the allowlist allows (priority 2): a
    // listed youtube.com can still have blocked channels, and study
    // mode can still restrict it to allowed channels. The listed
    // allows sit at 5, above the adult overlay (3) — the two never
    // overlap (adult domains are not YouTube), so this changes
    // nothing for adult blocking. These rules only match channel and
    // surface paths, never /playlist, so the Watch Later allow above
    // is not overridden; watch pages are never DNR-blocked by channel
    // rules (the content script enforces them).
    if (studyMode) {
      for (const pattern of YOUTUBE_SURFACE_PATTERNS)
        add('block', { regexFilter: pattern, resourceTypes: navigation }, 4);
      add(
        'block',
        { regexFilter: YOUTUBE_CHANNEL_CATCHALL, resourceTypes: navigation },
        4,
      );
      const seenAllowed = new Set();
      for (const key of allowedChannels) {
        if (seenAllowed.has(key)) continue;
        seenAllowed.add(key);
        add(
          'allow',
          { regexFilter: youtubeChannelRegex(key), resourceTypes: navigation },
          5,
        );
      }
    } else {
      const seenBlocked = new Set();
      for (const key of blockedChannels) {
        if (seenBlocked.has(key)) continue;
        seenBlocked.add(key);
        add(
          'block',
          { regexFilter: youtubeChannelRegex(key), resourceTypes: navigation },
          4,
        );
      }
    }
    if (blockEntertainment) {
      for (const pattern of YOUTUBE_ENTERTAINMENT_SURFACE_PATTERNS) {
        if (studyMode && !pattern.includes('/gaming')) continue;
        add('block', { regexFilter: pattern, resourceTypes: navigation }, 4);
      }
    }
  }
  return rules;
}
// Identify the browser this extension runs in, using the canonical id the
// desktop app uses for it (its macOS bundle id), so the app can tell
// whether the connected extension belongs to the browser the user picked.
export async function detectBrowser() {
  const ua = navigator.userAgent || '';
  try {
    if (navigator.brave && (await navigator.brave.isBrave()))
      return 'com.brave.browser';
  } catch {}
  const brands = (navigator.userAgentData?.brands ?? [])
    .map((b) => b.brand)
    .join(' ');
  if (/comet/i.test(brands)) return 'ai.perplexity.comet';
  if (/Comet/i.test(ua)) return 'ai.perplexity.comet';
  if (/Edg\//.test(ua)) return 'com.microsoft.edgemac';
  if (/Vivaldi\//.test(ua)) return 'com.vivaldi.vivaldi';
  if (/Chromium\//.test(ua)) return 'org.chromium.chromium';
  if (/Chrome\//.test(ua)) return 'com.google.chrome';
  return '';
}
export function leaseValid(policy, now = Date.now()) {
  return Boolean(
    policy &&
    Number.isFinite(policy.expires_at) &&
    policy.expires_at > now &&
    policy.expires_at <= now + 70_000,
  );
}
