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
  if (p.verification !== 'manual_checkpoints' && !setup?.native_available)
    return 'App checking is unavailable. Choose Manual check-ins in step 1.';
  if (network && !focus.browser_app) return 'Choose your browser below.';
  if (network && !setup?.browser.connected)
    return 'Connect the browser extension below, then check again.';
  if (network && setup?.browser.error) return 'Reconnect the extension below.';
  if (
    (p.verification !== 'manual_checkpoints' || focus.mode === 'strict') &&
    !(focus.mode === 'strict' ? focus.apps : p.work_apps).length &&
    !p.work_domains.length &&
    !(focus.mode === 'strict' && focus.domains.length)
  )
    return 'Choose at least one work app or website.';
  if (
    consent &&
    (p.verification !== 'manual_checkpoints' ||
      focus.apps.length > 0 ||
      focus.mode === 'strict') &&
    !focus.activity_consent
  )
    return 'Allow app activity below to continue.';
  if (
    consent &&
    p.verification === 'camera_and_activity' &&
    (!focus.camera || setup?.camera_permission !== 'granted')
  )
    return 'Allow camera access, or choose Apps only in step 1.';
  return '';
}
