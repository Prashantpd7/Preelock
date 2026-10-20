import { sameBrowser } from '../types';
import type { FocusConfig, FocusSetup } from '../types';
export const splitDomains = (value: string) =>
  value
    .split(/[\n,]/)
    .map((s) => s.trim())
    .filter(Boolean);
export function setupProblem(
  focus: FocusConfig,
  setup: FocusSetup | null,
  consent = true,
) {
  const p = focus.attention!;
  const network =
    (focus.mode === 'strict' && !!focus.browser_app) ||
    !focus.general_internet ||
    focus.domains.length > 0 ||
    p.work_domains.length > 0;
  if (!setup?.native_available)
    return 'App checking is unavailable on this device.';
  if (network && !focus.browser_app) return 'Choose your browser below.';
  // A connection from a different browser than the one picked — or one
  // that cannot say which browser it is in — does not count.
  const browserMismatch =
    !setup?.browser.client_browser ||
    !sameBrowser(focus.browser_app, setup.browser.client_browser);
  if (network && (!setup?.browser.connected || browserMismatch))
    return 'Connect the browser extension below, then check again.';
  if (network && setup?.browser.error) return 'Reconnect the extension below.';
  if (
    focus.website_blocking === 'allowlist' &&
    focus.domains.length === 0
  )
    return 'List at least one website to keep open, or pick a different blocking option.';
  if (
    !(focus.mode === 'strict' ? focus.apps : p.work_apps).length &&
    !p.work_domains.length &&
    !(focus.mode === 'strict' && focus.domains.length)
  )
    return 'Choose at least one work app or website.';
  if (consent && !focus.activity_consent)
    return 'Block all other apps below to continue.';
  if (
    consent &&
    p.verification === 'camera_and_activity' &&
    (!focus.camera || setup?.camera_permission !== 'granted')
  )
    return 'Allow camera access, or choose Apps only in step 1.';
  return '';
}
