import { expect, it } from 'vitest';
import { setupProblem } from '../services/focusSetup';
import { defaultFocus, type FocusSetup } from '../types';
const setup: FocusSetup = {
  platform: 'macos',
  native_available: true,
  applications: [],
  camera_permission: 'not_requested',
  pairing_token: '',
  browser: {
    available: true,
    connected: false,
    applied_id: '',
    error: '',
    blocks: 0,
    cleanup_pending: false,
    extension_stale: false,
  },
};
it('permits app-only Strict sessions without forcing extension setup', () => {
  const focus = {
    ...structuredClone(defaultFocus),
    mode: 'strict' as const,
    apps: ['com.microsoft.VSCode'],
    activity_consent: true,
  };
  expect(setupProblem(focus, setup)).toBe('');
});
it('requires consent and camera permission for camera-based focus', () => {
  const focus = {
    ...structuredClone(defaultFocus),
    activity_consent: false,
    attention: {
      ...defaultFocus.attention!,
      work_apps: ['com.microsoft.VSCode'],
      verification: 'camera_and_activity' as const,
    },
  };
  expect(setupProblem(focus, setup)).toContain('Block all other apps');
  focus.activity_consent = true;
  expect(setupProblem(focus, setup)).toContain('Allow camera');
  focus.camera = true;
  expect(setupProblem(focus, { ...setup, camera_permission: 'granted' })).toBe(
    '',
  );
});

it('requires a work choice even for Strict sessions', () => {
  const focus = {
    ...structuredClone(defaultFocus),
    mode: 'strict' as const,
    activity_consent: true,
    attention: {
      ...defaultFocus.attention!,
      verification: 'activity_only' as const,
    },
  };
  expect(setupProblem(focus, setup)).toContain('Choose at least one');
});
