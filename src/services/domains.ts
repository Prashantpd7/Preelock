// Website-rule input helpers: clean up what the user types so the strict
// backend validator rarely has to complain. Mirrors the Rust
// validate_rule() forms (see src-tauri/src/focus.rs): a domain
// (example.com, matching itself and subdomains), a wildcard
// (*.example.com, subdomains only), a domain with a path
// (example.com/shorts), or a search keyword (search:chess, which blocks
// search-engine result pages for that keyword). Scheme prefixes are
// stripped and hosts lowercased; paths and keywords are kept as typed
// (lowercased), never stripped.
const LABEL = '(?!-)[a-z0-9-]{1,63}(?<!-)';
const BARE_DOMAIN = new RegExp('^' + LABEL + '(\\.' + LABEL + ')+$');
const SINGLE_LABEL = new RegExp('^' + LABEL + '$');
// Clean a single website-rule line. Returns the stored form, or null
// when the line is not recognizable as a rule at all.
export function cleanDomain(raw: string): string | null {
  let s = raw.trim().toLowerCase();
  if (!s) return null;
  if (s.startsWith('search:')) {
    const keyword = s.slice(7).trim().replace(/\s+/g, ' ');
    if (
      keyword.length < 2 ||
      keyword.length > 100 ||
      !/^[a-z0-9][a-z0-9 _.'-]*$/.test(keyword)
    )
      return null;
    return 'search:' + keyword;
  }
  s = s.replace(/^[a-z][a-z0-9+.-]*:\/\//, '');
  // A pasted URL's query/fragment are not part of a rule; its path is.
  s = s.split(/[?#]/)[0];
  const slash = s.indexOf('/');
  let hostPart = slash === -1 ? s : s.slice(0, slash);
  let path = slash === -1 ? '' : '/' + s.slice(slash + 1).replace(/\/$/, '');
  if (path === '/') path = '';
  hostPart = hostPart.replace(/:\d+$/, '');
  if (hostPart.includes(':') || hostPart.includes('@')) return null;
  const wildcard = hostPart.startsWith('*.');
  const host = (wildcard ? hostPart.slice(2) : hostPart).replace(/\.$/, '');
  if (!host) return null;
  if (/^(\d+\.)+\d+$/.test(host)) return null;
  if (host.endsWith('.localhost') || host.endsWith('.local')) return null;
  if (wildcard) {
    if (!BARE_DOMAIN.test(host)) return null;
  } else if (BARE_DOMAIN.test(host)) {
    // Already a full domain.
  } else if (SINGLE_LABEL.test(host) && !path) {
    return host + '.com';
  } else {
    return null;
  }
  if (path && /[?#\s]/.test(path)) return null;
  return (wildcard ? '*.' : '') + host + path;
}
// Normalize a multi-line textarea. Bad lines are left untouched so the user
// can fix them; errors lists the offending lines.
export function normalizeDomainInput(value: string): {
  text: string;
  errors: string[];
} {
  const errors: string[] = [];
  const lines = value.split('\n').map((line) => {
    if (!line.trim()) return '';
    const cleaned = cleanDomain(line);
    if (cleaned === null) {
      errors.push(line.trim());
      return line;
    }
    return cleaned;
  });
  return { text: lines.join('\n'), errors };
}
// YouTube channel input helpers. Mirror the Rust
// normalize_youtube_channel() forms (see src-tauri/src/focus.rs) and
// browser-extension/rules.js: @handle (lowercase), a bare UC channel
// ID (case preserved), c:name / user:name (lowercase), or a YouTube
// channel-page URL. Video, search, playlist, Shorts, feed and embed
// URLs are rejected: they are not channels.
const HANDLE_BODY = /^[a-z0-9](?:[a-z0-9._-]*[a-z0-9])?$/;
const UC_ID = /^UC[A-Za-z0-9_-]{10,62}$/;
const LEGACY_NAME = /^[a-z0-9._-]+$/;
function cleanHandle(body: string): string | null {
  const lower = body.toLowerCase();
  if (lower.length < 3 || lower.length > 30 || !HANDLE_BODY.test(lower))
    return null;
  return '@' + lower;
}
function cleanLegacy(prefix: string, name: string): string | null {
  const lower = name.toLowerCase();
  if (lower.length < 1 || lower.length > 100 || !LEGACY_NAME.test(lower))
    return null;
  return prefix + lower;
}
// Clean a single channel line. Returns the stored key, or null when
// the line is not recognizable as a channel at all.
export function cleanYouTubeChannel(raw: string): string | null {
  const s = raw.trim();
  // eslint's no-control-regex forbids control characters in regex
  // literals, so check char codes directly instead.
  const hasControl = [...s].some((c) => {
    const code = c.charCodeAt(0);
    return code < 0x20 || code === 0x7f;
  });
  if (!s || hasControl || /\s/.test(s)) return null;
  if (s.startsWith('@')) return cleanHandle(s.slice(1));
  if (UC_ID.test(s)) return s;
  for (const prefix of ['c:', 'user:']) {
    if (s.startsWith(prefix))
      return cleanLegacy(prefix, s.slice(prefix.length));
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
  const segments = s
    .slice(schemeLen)
    .split(/[?#]/)[0]
    .split('/')
    .slice(1)
    .filter(Boolean);
  const first = segments[0];
  if (!first) return null;
  if (first.startsWith('@')) return cleanHandle(first.slice(1));
  if (first === 'channel' && segments[1] && UC_ID.test(segments[1]))
    return segments[1];
  if ((first === 'c' || first === 'user') && segments[1])
    return cleanLegacy(first + ':', segments[1]);
  return null;
}
// Normalize a multi-line channel textarea. Bad lines are left
// untouched so the user can fix them; errors lists the offenders.
export function normalizeYouTubeChannelInput(value: string): {
  text: string;
  errors: string[];
} {
  const errors: string[] = [];
  const lines = value.split('\n').map((line) => {
    if (!line.trim()) return '';
    const cleaned = cleanYouTubeChannel(line);
    if (cleaned === null) {
      errors.push(line.trim());
      return line;
    }
    return cleaned;
  });
  return { text: lines.join('\n'), errors };
}
// Live validation message for a channel textarea, without rewriting
// anything.
export function youTubeChannelInputError(value: string): string {
  const errors = normalizeYouTubeChannelInput(value).errors;
  if (!errors.length) return '';
  return (
    'Use one per line: @handle, a YouTube channel URL, or a UC ' +
    'channel ID. Invalid: ' +
    errors[0]
  );
}
// Live validation message for a textarea, without rewriting anything.
export function domainInputError(value: string): string {
  const errors = normalizeDomainInput(value).errors;
  if (!errors.length) return '';
  return (
    'Use one per line: a domain (example.com), *.example.com, a domain ' +
    'with a path (example.com/shorts), or search:keyword. Invalid: ' +
    errors[0]
  );
}
// App-blocking depth (step 4). Mirrors the Rust helpers in
// src-tauri/src/focus.rs (`normalize_path`, `is_system_folder`,
// `blocked_folders`, `title_rules`): folder matching normalizes
// separators to backslashes and compares case-insensitively, and
// system folders are rejected with a plain-language message.
function normalizeBlockedFolder(raw: string): string {
  let s = raw.trim();
  if (s.startsWith('\\\\?\\')) s = s.slice(4);
  else if (s.startsWith('//?/')) s = s.slice(4);
  s = s.replace(/\//g, '\\').toLowerCase();
  return s.replace(/\\+$/, '');
}
export function isSystemBlockedFolder(raw: string): boolean {
  const norm = normalizeBlockedFolder(raw);
  if (!norm) return false;
  const parts = norm.split('\\');
  // macOS: /System -> "\system".
  if (parts[0] === '' && parts[1] === 'system') return true;
  // Windows: <drive>:\Windows, or any System32/SysWOW64 component.
  if (parts.length >= 2 && parts[1] === 'windows') return true;
  return parts.some((p) => p === 'system32' || p === 'syswow64');
}
// Live validation message for the blocked-folders textarea, without
// rewriting anything. Empty lines are fine (skipped on submit).
export function blockedFolderInputError(value: string): string {
  for (const line of value.split('\n')) {
    const folder = line.trim();
    if (!folder) continue;
    const hasControl = [...folder].some((c) => {
      const code = c.charCodeAt(0);
      return code < 0x20 || code === 0x7f;
    });
    if (hasControl || folder.length > 1024)
      return `Invalid blocked folder: ${folder}`;
    if (folder === '/' || folder === '\\' || /^[a-z]:[\\/]?$/i.test(folder))
      return `A whole drive or filesystem root cannot be blocked: ${folder}`;
    if (isSystemBlockedFolder(folder))
      return `System folders cannot be blocked: ${folder}`;
  }
  return '';
}
// Live validation message for the window-title rules textarea.
export function blockedTitleInputError(value: string): string {
  for (const line of value.split('\n')) {
    const rule = line.trim();
    if (!rule) continue;
    const hasControl = [...rule].some((c) => {
      const code = c.charCodeAt(0);
      return code < 0x20 || code === 0x7f;
    });
    if (hasControl || rule.length > 100)
      return `Invalid window-title rule: ${rule}`;
  }
  return '';
}
