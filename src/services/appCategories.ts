// Categorization for the Screen Time list: a display-only label shown
// below each app/website name (Distracting / Productive / Others).
// There is no stored per-app classification in the codebase (session
// attention metrics track productive/distracted time, not per-app
// labels). The built-in mapping below is the default; the user's own
// corrections (category_overrides in preferences) are checked FIRST and
// win wherever the Screen Time page categorizes usage — list, chart,
// and totals alike — including for past days, because categories are
// applied at render time from this one helper. Session behavior never
// reads this helper, so overrides cannot change blocking or timers.

export type AppCategory = 'Distracting' | 'Productive' | 'Others';

// User corrections, keyed by categoryKey(): an app's bundle id (or file
// path for bare executables) or a website's domain.
export type CategoryOverrides = Record<string, AppCategory>;

const VALID_CATEGORIES: ReadonlySet<string> = new Set([
  'Distracting',
  'Productive',
  'Others',
]);

// Checked first for apps: any of these tokens means a browser.
const BROWSER_TOKENS = new Set([
  'browser',
  'chrome',
  'chromium',
  'safari',
  'firefox',
  'brave',
  'comet',
  'arc',
  'edge',
  'opera',
  'vivaldi',
  'duckduckgo',
  'dia',
  'orion',
]);

const DISTRACTING_TOKENS = new Set([
  'instagram',
  'facebook',
  'snapchat',
  'tiktok',
  'twitter',
  'reddit',
  'youtube',
  'netflix',
  'twitch',
  'steam',
  'minecraft',
  'roblox',
  'telegram',
  'discord',
  'messenger',
  'tinder',
  'bumble',
  'hinge',
  'pinterest',
  'threads',
]);

const PRODUCTIVE_TOKENS = new Set([
  'preelock',
  'muse',
  'freebuff',
  'terminal',
  'iterm',
  'warp',
  'kitty',
  'alacritty',
  'code',
  'xcode',
  'cursor',
  'zed',
  'neovim',
  'vim',
  'emacs',
  'intellij',
  'pycharm',
  'webstorm',
  'sublime',
  'vscode',
  'androidstudio',
  'github',
  'gitkraken',
  'sourcetree',
  'docker',
  'postman',
  'figma',
  'notion',
  'obsidian',
  'bear',
  'evernote',
  'linear',
  'slack',
  'zoom',
  'chatgpt',
  'claude',
  'copilot',
  'gemini',
  'word',
  'excel',
  'powerpoint',
  'keynote',
  'pages',
  'numbers',
  'docs',
  'sheets',
  'slides',
]);

const DISTRACTING_DOMAINS = [
  'youtube.com',
  'instagram.com',
  'facebook.com',
  'twitter.com',
  'x.com',
  'tiktok.com',
  'snapchat.com',
  'reddit.com',
  'netflix.com',
  'twitch.tv',
  'disneyplus.com',
  'primevideo.com',
  'hulu.com',
  'pinterest.com',
  'threads.net',
  'discord.com',
  'telegram.org',
  'store.steampowered.com',
  'steamcommunity.com',
];

const PRODUCTIVE_DOMAINS = [
  'github.com',
  'stackoverflow.com',
  'stackexchange.com',
  'developer.mozilla.org',
  'wikipedia.org',
  'chatgpt.com',
  'openai.com',
  'claude.ai',
  'anthropic.com',
  'perplexity.ai',
  'notion.so',
  'notion.com',
  'figma.com',
  'linear.app',
  'coursera.org',
  'udemy.com',
  'khanacademy.org',
  'w3schools.com',
  'geeksforgeeks.org',
  'docs.google.com',
  'drive.google.com',
];

function appTokens(name: string, target: string): Set<string> {
  const tokens = new Set<string>();
  for (const part of `${name} ${target}`.toLowerCase().split(/[^a-z0-9]+/)) {
    if (part) tokens.add(part);
  }
  return tokens;
}

function domainOf(target: string): string {
  return target
    .toLowerCase()
    .replace(/^www\./, '')
    .split('/')[0]
    .split('?')[0];
}

function domainMatches(domain: string, list: string[]): boolean {
  return list.some((d) => domain === d || domain.endsWith(`.${d}`));
}

// Stable identity for one override entry: a website's domain, or an
// app's target exactly as recorded (bundle id, or file path for bare
// executables such as the dev binary).
export function categoryKey(
  kind: 'app' | 'site',
  target: string,
): string {
  return kind === 'site' ? domainOf(target) : target;
}

export function appCategory(
  kind: 'app' | 'site',
  target: string,
  name: string,
  overrides?: CategoryOverrides,
): AppCategory {
  const override = overrides?.[categoryKey(kind, target)];
  if (override && VALID_CATEGORIES.has(override)) return override;
  if (kind === 'site') {
    const domain = domainOf(target || name);
    if (domainMatches(domain, DISTRACTING_DOMAINS)) return 'Distracting';
    if (domainMatches(domain, PRODUCTIVE_DOMAINS)) return 'Productive';
    return 'Others';
  }
  const tokens = appTokens(name, target);
  const has = (list: Set<string>) =>
    [...list].some((token) => tokens.has(token));
  if (has(BROWSER_TOKENS)) return 'Others';
  if (has(DISTRACTING_TOKENS)) return 'Distracting';
  if (has(PRODUCTIVE_TOKENS)) return 'Productive';
  return 'Others';
}
