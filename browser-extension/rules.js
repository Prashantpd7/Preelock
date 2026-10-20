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
  const safe = policy.domains.filter(
    (d) => /^[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?$/.test(d) && d.includes('.'),
  );
  if (safe.length !== policy.domains.length)
    throw new Error('Invalid policy domains');
  if (policy.mode === 'blocklist')
    return safe.map((domain, index) => ({
      id: index + 1,
      priority: 1,
      action: { type: 'block' },
      condition: { requestDomains: [domain], resourceTypes: types },
    }));
  if (!['allowlist', 'offline'].includes(policy.mode))
    throw new Error('Unknown network policy');
  // Navigation types only. An allowed page must load its own styles,
  // scripts, images and fonts (often served from other domains, e.g.
  // github.githubassets.com) or it renders broken. Subresource requests
  // are therefore never filtered in allowlist mode; only opening a page
  // (top-level or iframe) outside the allowed list is blocked.
  const navigation = ['main_frame', 'sub_frame'];
  // Allowlist is implemented as explicit ALLOW rules (higher priority) plus
  // a catch-all BLOCK rule, instead of a single rule with
  // excludedRequestDomains. This uses only requestDomains + regexFilter,
  // the same primitives the working blocklist mode uses.
  const allowed =
    policy.mode === 'allowlist' ? [...local, ...safe] : [...local];
  const rules = allowed.map((domain, index) => ({
    id: index + 1,
    priority: 2,
    action: { type: 'allow' },
    condition: {
      requestDomains: [domain],
      resourceTypes: policy.mode === 'offline' ? types : navigation,
    },
  }));
  rules.push({
    id: rules.length + 1,
    priority: 1,
    action: { type: 'block' },
    condition: {
      regexFilter: '^(https?|wss?)://',
      // Offline blocks every request type; allowlist blocks navigation
      // only so allowed pages can load their styles, scripts and images.
      resourceTypes: policy.mode === 'offline' ? types : navigation,
    },
  });
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
