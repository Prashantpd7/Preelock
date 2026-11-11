import type { FocusConfig } from '../types';

/** Safari's app id. Preelock has no Safari extension, so Safari can never
 *  enforce a session's website rules. It stays a normal application
 *  everywhere else: screen-time tracking, app blocking, everyday use. */
export const SAFARI_APP_ID = 'com.apple.Safari';

export function isSafariId(id: string | null | undefined): boolean {
  const value = (id ?? '').trim().toLowerCase();
  return (
    value === SAFARI_APP_ID.toLowerCase() || value.endsWith('\\safari.exe')
  );
}

/** Official download pages for the browsers Preelock can enforce rules in.
 *  Used only when no supported browser is installed; nothing is
 *  downloaded or installed by Preelock. */
export const SUPPORTED_DOWNLOADS: { id: string; name: string }[] = [
  { id: 'com.google.Chrome', name: 'Chrome' },
  { id: 'com.microsoft.edgemac', name: 'Edge' },
  { id: 'com.brave.Browser', name: 'Brave' },
  { id: 'com.vivaldi.Vivaldi', name: 'Vivaldi' },
  { id: 'ai.perplexity.comet', name: 'Comet' },
];

/**
 * True when the session's website rules must be enforced by the browser
 * extension. Mirrors the wizard's own pre-submit gate and
 * `FocusConfig::network_required` in focus.rs; a session with no website
 * rules (and no offline / frozen enforcement) needs no extension, so
 * Safari is a perfectly normal app there.
 */
export function sessionNeedsExtension(focus: FocusConfig): boolean {
  // An explicit blocking choice counts immediately: the user has asked for
  // extension-enforced rules even before any site is typed.
  const chosenBlocking =
    focus.website_blocking === 'blocklist' ||
    focus.website_blocking === 'allowlist';
  return (
    chosenBlocking ||
    !focus.general_internet ||
    focus.domains.length > 0 ||
    (focus.mode !== 'standard' && !!focus.browser_app) ||
    (focus.attention?.work_domains.length ?? 0) > 0 ||
    (focus.youtube_blocked_channels?.length ?? 0) > 0 ||
    (focus.youtube_allowed_channels?.length ?? 0) > 0 ||
    !!focus.youtube_study_mode ||
    (!!focus.frozen?.enabled && !!focus.browser_app)
  );
}
