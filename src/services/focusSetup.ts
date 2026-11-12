import { sameBrowser, sessionBrowsers } from '../types';
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
    p.work_domains.length > 0 ||
    // YouTube channel rules need the extension too, so a session
    // with only channel rules still requires a browser connection.
    // The Watch-Later toggle alone is not a rule (see
    // FocusConfig::network_required in focus.rs).
    (focus.youtube_blocked_channels?.length ?? 0) > 0 ||
    (focus.youtube_allowed_channels?.length ?? 0) > 0 ||
    !!focus.youtube_study_mode ||
    !!focus.youtube_block_entertainment;
  // Step 12: a frozen session with a picked browser needs that
  // browser connected (the backend preflight requires it), but a
  // frozen session with no picked browser does not need one
  // chosen — its policy applies to any browser, and the sampler
  // blocks browser apps not in the frozen allowlist anyway.
  const frozenNetwork = !!focus.frozen?.enabled && !!focus.browser_app;
  const needsConnection = network || frozenNetwork;
  if (!setup?.native_available)
    return 'App checking is unavailable on this device.';
  if (network && !focus.browser_app)
    return 'No browser detected — open Comet, Chrome, Edge, Brave or Vivaldi, then refresh apps.';
  // A connection from a different browser than the one picked — or one
  // that cannot say which browser it is in — does not count.
  const browserMismatch =
    !setup?.browser.client_browser ||
    !sameBrowser(focus.browser_app, setup.browser.client_browser);
  if (needsConnection && (!setup?.browser.connected || browserMismatch))
    return 'Connect the browser extension below, then check again.';
  if (needsConnection && setup?.browser.error)
    return 'Reconnect the extension below.';
  if (focus.website_blocking === 'allowlist' && focus.domains.length === 0)
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

/** Which fixable prerequisites a session is still missing, as
 *  separate flags (unlike `setupProblem`, which returns only the
 *  first problem as text): the pre-session warning shows the
 *  browser and camera fixes side by side when both are missing.
 *  The rules mirror `setupProblem` and the manual "Before you
 *  start" dialog, so the warning and the manual start agree:
 *  - Browser: the session needs website blocking (domains, blocked
 *    internet, strict mode with a browser, work domains, YouTube
 *    channel rules, or a frozen session with a picked browser) and
 *    its browser is not properly connected (not connected, an
 *    error, or the extension reporting a different browser).
 *  - Camera: the session needs camera checks (`camera` on, or
 *    `camera_and_activity` verification) and access is not granted.
 *    Camera checks exist on macOS only, so a camera requirement
 *    never counts as missing on Windows.
 *  No setup yet, or a device without native checking, means
 *  nothing is reported missing: the caller shows "Checking…"
 *  instead of a fix it cannot verify. */
export interface MissingPrerequisites {
  browser: boolean;
  camera: boolean;
}

export function missingPrerequisites(
  focus: FocusConfig,
  setup: FocusSetup | null,
): MissingPrerequisites {
  const none = { browser: false, camera: false };
  if (!setup || !setup.native_available) return none;
  const p = focus.attention!;
  const network =
    (focus.mode === 'strict' && !!focus.browser_app) ||
    !focus.general_internet ||
    focus.domains.length > 0 ||
    p.work_domains.length > 0 ||
    (focus.youtube_blocked_channels?.length ?? 0) > 0 ||
    (focus.youtube_allowed_channels?.length ?? 0) > 0 ||
    !!focus.youtube_study_mode ||
    !!focus.youtube_block_entertainment;
  const frozenNetwork = !!focus.frozen?.enabled && !!focus.browser_app;
  const needsConnection = network || frozenNetwork;
  const sessionBrowserIds = sessionBrowsers(focus);
  const connectedIds = setup.connected_browsers ?? [];
  const unpairedBrowsers = sessionBrowserIds.filter(
    (id) => !connectedIds.some((c) => sameBrowser(c, id)),
  );
  const browserOk =
    !!setup.browser.connected &&
    !setup.browser.error &&
    (!focus.browser_app ||
      (!!setup.browser.client_browser &&
        sameBrowser(focus.browser_app, setup.browser.client_browser))) &&
    unpairedBrowsers.length === 0;
  const needsCamera =
    focus.camera === true || p.verification === 'camera_and_activity';
  return {
    browser: needsConnection && !browserOk,
    camera:
      needsCamera &&
      setup.platform !== 'windows' &&
      setup.camera_permission !== 'granted',
  };
}
