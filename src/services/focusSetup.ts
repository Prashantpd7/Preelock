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
  if (!setup?.native_available)
    return 'App checking is unavailable on this device.';
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
  const needsCamera =
    focus.camera === true || p.verification === 'camera_and_activity';
  return {
    // Browser extension connection is optional: native DNS/hosts/AX enforcement
    // handles website blocking without requiring a paired browser extension.
    browser: false,
    camera:
      needsCamera &&
      setup.platform !== 'windows' &&
      setup.camera_permission !== 'granted',
  };
}
