// Extension-local Website Controls (functional adult & distracting site
// categories enforced independently of Preelock desktop session state).
export const WEBSITE_CONTROLS_KEY = 'websiteControls';

export const WEBSITE_CONTROL_DEFINITIONS = [
  {
    key: 'blockAdultWebsites',
    label: 'Block Adult Websites',
    description: 'Block access to known adult websites while browsing.',
  },
  {
    key: 'blockDistractingWebsites',
    label: 'Block Distracting Websites',
    description:
      'Block distracting sites such as Instagram, Facebook and Netflix.',
  },
];

export const DEFAULT_WEBSITE_CONTROLS = Object.freeze({
  blockAdultWebsites: false,
  blockDistractingWebsites: false,
});

// Initial distracting platforms (sorted canonically for DNR comparison).
// Explicitly excludes YouTube and WhatsApp.
export const DISTRACTING_DOMAINS = Object.freeze([
  'facebook.com',
  'instagram.com',
  'netflix.com',
]);

// General-purpose communication and study platforms that extension-local
// Website Controls must never block (YouTube has its own dedicated controls
// and desktop rules; WhatsApp must stay accessible unless desktop blocks it).
export const WEBSITE_CONTROLS_EXCLUDED_DOMAINS = Object.freeze([
  'youtube.com',
  'youtu.be',
  'whatsapp.com',
]);

export const DISTRACTING_DOMAIN_SET = new Set(DISTRACTING_DOMAINS);

// Dedicated DNR dynamic rule ID ranges, completely disjoint from session rules
// (1..N) so session start/end/reconnect never remove Website Controls rules.
export const DISTRACTING_RULE_ID = 100001;
export const ADULT_RULE_ID_BASE = 101000;
export const ADULT_RULE_ID_MAX = 101999;
export const ADULT_RULE_CHUNK_SIZE = 5000;

// Priority above all session allow/exception rules (2, 5) so enabled local
// category restrictions always enforce additive blocking.
export const WEBSITE_CONTROL_RULE_PRIORITY = 6;

export const WEBSITE_CONTROL_RESOURCE_TYPES = Object.freeze([
  'main_frame',
  'sub_frame',
  'xmlhttprequest',
  'media',
  'websocket',
]);

export function normalizeWebsiteControls(raw) {
  const input = raw && typeof raw === 'object' ? raw : {};
  return {
    blockAdultWebsites: Boolean(input.blockAdultWebsites),
    blockDistractingWebsites: Boolean(input.blockDistractingWebsites),
  };
}

export function countActiveWebsiteControls(controls) {
  const normalized = normalizeWebsiteControls(controls);
  return (
    (normalized.blockAdultWebsites ? 1 : 0) +
    (normalized.blockDistractingWebsites ? 1 : 0)
  );
}

export function isWebsiteControlRuleId(id) {
  return (
    id === DISTRACTING_RULE_ID ||
    (id >= ADULT_RULE_ID_BASE && id <= ADULT_RULE_ID_MAX)
  );
}

export function isExcludedWebsiteControlsHost(hostname) {
  const host = String(hostname || '')
    .toLowerCase()
    .replace(/\.$/, '');
  if (!host) return false;
  return WEBSITE_CONTROLS_EXCLUDED_DOMAINS.some(
    (excluded) => host === excluded || host.endsWith('.' + excluded),
  );
}

export function hostMatchesDomainSet(hostname, domainSet) {
  const host = String(hostname || '')
    .toLowerCase()
    .replace(/\.$/, '');
  if (!host || !domainSet || domainSet.size === 0) return false;
  const labels = host.split('.');
  for (let i = 0; i < labels.length - 1; i++) {
    const candidate = labels.slice(i).join('.');
    if (domainSet.has(candidate)) return true;
  }
  return false;
}

// Determine which local Website Controls category blocks `urlString`, or null.
export function websiteControlBlockCategory(
  urlString,
  controls,
  adultDomainSet = null,
) {
  const normalized = normalizeWebsiteControls(controls);
  if (!normalized.blockAdultWebsites && !normalized.blockDistractingWebsites) {
    return null;
  }
  let url;
  try {
    url = new URL(urlString || '');
  } catch {
    return null;
  }
  if (!/^https?:$/.test(url.protocol)) return null;
  const hostname = url.hostname.toLowerCase().replace(/\.$/, '');
  if (!hostname || ['localhost', '127.0.0.1', '[::1]'].includes(hostname)) {
    return null;
  }
  if (isExcludedWebsiteControlsHost(hostname)) {
    return null;
  }
  if (
    normalized.blockAdultWebsites &&
    adultDomainSet &&
    hostMatchesDomainSet(hostname, adultDomainSet)
  ) {
    return 'website_adult';
  }
  if (
    normalized.blockDistractingWebsites &&
    hostMatchesDomainSet(hostname, DISTRACTING_DOMAIN_SET)
  ) {
    return 'website_distracting';
  }
  return null;
}

// Compile desired Chromium declarativeNetRequest dynamic rules for active local
// Website Controls. Uses chunked requestDomains rules (10 rules for ~47k domains)
// with excluded platform domains omitted by list construction and safelisting.
export function compileWebsiteControlRules(controls, adultDomains = []) {
  const normalized = normalizeWebsiteControls(controls);
  const rules = [];
  const resourceTypes = [...WEBSITE_CONTROL_RESOURCE_TYPES];

  if (normalized.blockAdultWebsites && adultDomains.length > 0) {
    for (let i = 0; i < adultDomains.length; i += ADULT_RULE_CHUNK_SIZE) {
      const chunkIndex = Math.floor(i / ADULT_RULE_CHUNK_SIZE);
      const id = ADULT_RULE_ID_BASE + chunkIndex;
      if (id > ADULT_RULE_ID_MAX) break;
      const chunk = adultDomains.slice(i, i + ADULT_RULE_CHUNK_SIZE);
      if (!chunk.length) continue;
      rules.push({
        id,
        priority: WEBSITE_CONTROL_RULE_PRIORITY,
        action: { type: 'block' },
        condition: {
          requestDomains: chunk,
          resourceTypes,
        },
      });
    }
  }

  if (normalized.blockDistractingWebsites) {
    rules.push({
      id: DISTRACTING_RULE_ID,
      priority: WEBSITE_CONTROL_RULE_PRIORITY,
      action: { type: 'block' },
      condition: {
        requestDomains: [...DISTRACTING_DOMAINS],
        resourceTypes,
      },
    });
  }

  return rules;
}
