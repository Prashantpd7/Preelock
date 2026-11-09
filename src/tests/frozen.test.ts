import { describe, expect, it } from 'vitest';
import { defaultFocus, frozenLabel, type FocusConfig } from '../types';
import { setupProblem } from '../services/focusSetup';
import type { FocusSetup } from '../types';

const setup: FocusSetup = {
  platform: 'macos',
  native_available: true,
  applications: [],
  camera_permission: 'granted',
  browser: {
    available: true,
    connected: false,
    applied_id: '',
    client_browser: '',
    error: '',
    blocks: 0,
    cleanup_pending: false,
    extension_stale: false,
  },
  connected_browsers: [],
  pairing_token: '',
  extension_directory: '',
};

describe('frozenLabel', () => {
  it('is null when not frozen', () => {
    expect(frozenLabel(defaultFocus)).toBeNull();
    expect(frozenLabel(undefined)).toBeNull();
  });
  it('describes the default frozen allowlists', () => {
    const focus: FocusConfig = {
      ...defaultFocus,
      frozen: { enabled: true, allowed_apps: [], allowed_websites: [] },
    };
    expect(frozenLabel(focus)).toBe(
      'only Preelock + system apps · no websites allowed',
    );
  });
  it('counts listed apps and sites', () => {
    const focus: FocusConfig = {
      ...defaultFocus,
      frozen: {
        enabled: true,
        allowed_apps: ['com.microsoft.VSCode'],
        allowed_websites: ['docs.example.com', 'en.wikipedia.org'],
      },
    };
    expect(frozenLabel(focus)).toBe('1 app allowed · 2 sites allowed');
  });
});

describe('setupProblem with frozen sessions', () => {
  it('does not require a browser for a frozen session with none picked', () => {
    const focus: FocusConfig = {
      ...defaultFocus,
      frozen: { enabled: true, allowed_apps: [], allowed_websites: [] },
    };
    // The only remaining problem is the work-app requirement, not
    // a browser requirement: the message must not mention browsers.
    const problem = setupProblem(focus, setup);
    expect(problem).not.toContain('browser');
  });
  it('requires the picked browser to be connected for a frozen session', () => {
    const focus: FocusConfig = {
      ...defaultFocus,
      browser_app: 'com.google.Chrome',
      frozen: { enabled: true, allowed_apps: [], allowed_websites: [] },
    };
    expect(setupProblem(focus, setup)).toContain('Connect the browser');
  });
});
