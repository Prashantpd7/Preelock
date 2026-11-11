// Browser identity for this extension installation.
//
// There is no single reliable API: Chromium browsers inherit most of
// each other's signals, and a service worker has fewer of them than a
// document (Brave exposes navigator.brave only to documents, and every
// Brave build sends Chrome's user agent). So identity is resolved from
// the strongest available signal, and the answer is '' — "not
// identified" — whenever the signals cannot tell two supported
// browsers apart. The desktop app refuses an unidentified connection
// rather than registering it under the wrong browser.
//
// Signals, strongest first:
// 1. navigator.brave.isBrave() — definitive, document contexts only.
// 2. User-Agent Client Hints brands — Comet and Edge name themselves.
// 3. User-Agent tokens — Vivaldi (Vivaldi/), Opera (OPR/, unsupported),
//    Chromium (Chromium/).
// 4. In a document, the Brave probe is conclusive (Brave always
//    exposes the API), so a Chrome user agent there is really Chrome.
//    In a worker the same user agent may be Brave or Chrome, so it is
//    left unidentified and the identity the popup detected is used.
export const browserNames = {
  'ai.perplexity.comet': 'Comet',
  'com.google.chrome': 'Chrome',
  'com.microsoft.edgemac': 'Edge',
  'com.brave.browser': 'Brave',
  'com.vivaldi.vivaldi': 'Vivaldi',
  'org.chromium.chromium': 'Chromium',
};

export const supportedBrowsers = Object.keys(browserNames);

export function isSupportedBrowser(id) {
  return Object.hasOwn(browserNames, id ?? '') && Boolean(id);
}

// Pure resolver, so every signal combination is testable without a
// browser. `signals.brave` is true/false/undefined; `signals.brands`
// is navigator.userAgentData.brands (strings or {brand} objects);
// `signals.userAgent` is the UA string. `trusted` means the Brave
// probe was conclusive (a document context), not merely that it was
// unavailable.
export function detectFromSignals(signals = {}, trusted = false) {
  const userAgent = String(signals.userAgent || '');
  const brands = (signals.brands || [])
    .map((entry) => String(entry && entry.brand ? entry.brand : entry))
    .filter(Boolean);
  const brand = (re) => brands.some((name) => re.test(name));
  if (signals.brave === true) return 'com.brave.browser';
  if (brand(/comet/i) || /\bComet\//i.test(userAgent))
    return 'ai.perplexity.comet';
  if (brand(/microsoft edge/i) || /\bEdg[A-Z]?\//i.test(userAgent))
    return 'com.microsoft.edgemac';
  if (/Vivaldi\//i.test(userAgent)) return 'com.vivaldi.vivaldi';
  // Opera is Chromium-based but not supported: never report it as
  // Chrome, so the desktop app can refuse it honestly.
  if (/\bOPR\/|\bOpera[ /]/i.test(userAgent)) return '';
  if (/\bChromium\//i.test(userAgent) && !/\bChrome\//i.test(userAgent))
    return 'org.chromium.chromium';
  if (trusted && /\bChrome\//i.test(userAgent)) {
    // Only a document can rule Brave out. Between Chrome and Chromium,
    // the brand list decides: Chromium names itself, Chrome names
    // "Google Chrome". Without any brand list, Chrome is the honest
    // answer for a Chrome user agent.
    if (brand(/google chrome/i)) return 'com.google.chrome';
    if (brand(/^chromium$/i)) return 'org.chromium.chromium';
    return 'com.google.chrome';
  }
  return '';
}

async function probe(trusted) {
  let brave;
  try {
    if (typeof navigator !== 'undefined' && navigator.brave)
      brave = Boolean(await navigator.brave.isBrave());
  } catch {
    brave = undefined;
  }
  let brands = [];
  try {
    brands = (navigator.userAgentData && navigator.userAgentData.brands) || [];
  } catch {
    brands = [];
  }
  const userAgent =
    (typeof navigator !== 'undefined' && navigator.userAgent) || '';
  return detectFromSignals({ brave, brands, userAgent }, trusted);
}

// Document contexts (the popup): the Brave probe is available, so a
// Chrome-shaped browser that is not Brave is Chrome.
export function detectBrowser() {
  return probe(true);
}

// Worker contexts: only the signals that are conclusive on their own
// are used; an ambiguous Chrome-family user agent resolves to ''.
export function detectBrowserInWorker() {
  return probe(false);
}

// The identity to report to the desktop app. The popup detects in a
// document and stores the result; the worker prefers what its own
// context can prove, then that stored identity. `browser_choice` is
// only a compatibility fallback for browsers that paired before
// automatic detection existed.
export async function browserIdentity() {
  const detected = await detectBrowserInWorker();
  if (detected) return detected;
  const { browser_identity, browser_choice } = await chrome.storage.local.get([
    'browser_identity',
    'browser_choice',
  ]);
  if (isSupportedBrowser(browser_identity)) return browser_identity;
  if (isSupportedBrowser(browser_choice)) return browser_choice;
  return '';
}
