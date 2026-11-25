import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { Sessions } from '../pages/Sessions';
import { useAppStore } from '../state/store';
import { api, emptySnapshot } from '../services/api';
import { isSafariId, sessionNeedsExtension } from '../services/browserSupport';
import { defaultFocus, type FocusSetup } from '../types';

// The desktop API is mocked: these tests exercise the session wizard's
// browser validation against a fabricated browser/connection state. Real
// extension pairing is covered by the Rust bridge tests and on-device use.
vi.mock('../services/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../services/api')>();
  return {
    ...actual,
    desktop: true,
    api: {
      ...actual.api,
      focusSetup: vi.fn(),
      openSetup: vi.fn(),
      resetPairing: vi.fn(),
      snapshot: vi.fn(),
      execute: vi.fn(),
    },
  };
});

const mocked = vi.mocked(api);

const connectedChrome = {
  available: true,
  connected: true,
  applied_id: '',
  client_browser: 'com.google.Chrome',
  error: '',
  blocks: 0,
  cleanup_pending: false,
  extension_stale: false,
};
const notConnected = {
  ...connectedChrome,
  connected: false,
  client_browser: '',
};

function setupFixture(over: Partial<FocusSetup> = {}): FocusSetup {
  return {
    platform: 'macos',
    native_available: true,
    applications: [
      { id: 'com.apple.Safari', name: 'Safari' },
      { id: 'com.google.Chrome', name: 'Google Chrome' },
      { id: 'com.brave.Browser', name: 'Brave' },
      { id: 'com.microsoft.VSCode', name: 'Visual Studio Code' },
    ],
    browsers: [
      { id: 'com.google.Chrome', name: 'Google Chrome' },
      { id: 'com.brave.Browser', name: 'Brave' },
    ],
    connected_browsers: ['com.google.Chrome'],
    extension_directory: '/tmp/preelock-extension',
    pairing_token: 'pair-me',
    camera_permission: 'granted',
    browser: connectedChrome,
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocked.focusSetup.mockImplementation(async (browserId?: string) =>
    setupFixture(
      browserId === 'com.brave.Browser'
        ? { browser: notConnected, connected_browsers: [] }
        : {},
    ),
  );
  mocked.openSetup.mockResolvedValue(undefined);
  mocked.resetPairing.mockResolvedValue(undefined);
  mocked.snapshot.mockResolvedValue(structuredClone(emptySnapshot));
  mocked.execute.mockResolvedValue(structuredClone(emptySnapshot));
  useAppStore.setState({
    data: structuredClone(emptySnapshot),
    loading: false,
    error: null,
    busy: false,
  });
});

/** Opens the wizard, fills the task step and lands on Apps & websites. */
async function openAppsStep(entry: RegExp = /New session/) {
  render(<Sessions />);
  fireEvent.click(screen.getByRole('button', { name: entry }));
  fireEvent.change(screen.getByLabelText('Task name'), {
    target: { value: 'Deep work' },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
  await screen.findByLabelText('Safari');
}

const enableWebsiteBlocking = () => {
  fireEvent.click(screen.getByLabelText('Website blocking'));
  fireEvent.click(screen.getByLabelText('Block distracting websites'));
};
const selectSafari = () => fireEvent.click(screen.getByLabelText('Safari'));

describe('Safari is supported for session website blocking', () => {
  it('does not show any restriction notice and keeps Safari selected when website blocking is enabled', async () => {
    await openAppsStep();
    enableWebsiteBlocking();
    selectSafari();
    expect(
      screen.queryByText("Safari isn't supported yet"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText(
        'Preelock needs a supported browser to block websites during your session.',
      ),
    ).not.toBeInTheDocument();
    expect(screen.getByLabelText('Safari')).toBeChecked();
  });

  it('applies to scheduled and one-shot sessions too without restriction notice', async () => {
    await openAppsStep(/New schedule/);
    enableWebsiteBlocking();
    selectSafari();
    expect(
      screen.queryByText("Safari isn't supported yet"),
    ).not.toBeInTheDocument();
    expect(screen.getByLabelText('Safari')).toBeChecked();
  });

  it('disables YouTube blocking and shows warning when Safari is selected', async () => {
    await openAppsStep();
    selectSafari();
    const ytCheckbox = screen.getByLabelText('YouTube blocking');
    expect(ytCheckbox).toBeDisabled();
    fireEvent.click(ytCheckbox);
    expect(
      screen.getByText('YouTube blocking is not available on Safari yet.'),
    ).toBeInTheDocument();
  });

  it('keeps other application selections when Safari is selected', async () => {
    await openAppsStep();
    fireEvent.click(screen.getByLabelText('Visual Studio Code'));
    enableWebsiteBlocking();
    selectSafari();
    expect(
      screen.queryByText("Safari isn't supported yet"),
    ).not.toBeInTheDocument();
    expect(screen.getByLabelText('Safari')).toBeChecked();
    expect(screen.getByLabelText('Visual Studio Code')).toBeChecked();
  });

  it('never appears outside session configuration', () => {
    useAppStore.setState({
      data: {
        ...structuredClone(emptySnapshot),
        sessions: [
          {
            id: 's1',
            title: 'Yesterday',
            duration_minutes: 25,
            remaining_ms: 0,
            focused_ms: 0,
            allow_pause: true,
            status: 'completed',
            started_at: Date.now() - 3_600_000,
            ended_at: null,
            deadline_ms: null,
            focus: {
              ...defaultFocus,
              domains: ['youtube.com'],
              attention: {
                ...defaultFocus.attention!,
                work_apps: ['com.apple.Safari'],
              },
            },
          },
        ],
      },
    });
    render(<Sessions />);
    expect(
      screen.queryByText("Safari isn't supported yet"),
    ).not.toBeInTheDocument();
    expect(screen.getByText('Yesterday')).toBeInTheDocument();
  });
});

describe('sessionNeedsExtension', () => {
  it('is false for a session with no website rules', () => {
    expect(sessionNeedsExtension(defaultFocus)).toBe(false);
  });

  it('is true for the blocking choices and other extension-only rules', () => {
    expect(
      sessionNeedsExtension({ ...defaultFocus, website_blocking: 'blocklist' }),
    ).toBe(true);
    expect(
      sessionNeedsExtension({
        ...defaultFocus,
        website_blocking: 'allowlist',
        mode: 'strict',
      }),
    ).toBe(true);
    expect(
      sessionNeedsExtension({ ...defaultFocus, domains: ['youtube.com'] }),
    ).toBe(true);
    expect(
      sessionNeedsExtension({
        ...defaultFocus,
        youtube_study_mode: true,
      }),
    ).toBe(true);
    expect(
      sessionNeedsExtension({ ...defaultFocus, general_internet: false }),
    ).toBe(true);
  });

  it('recognizes Safari ids and only Safari ids', () => {
    expect(isSafariId('com.apple.Safari')).toBe(true);
    expect(isSafariId('COM.APPLE.SAFARI')).toBe(true);
    expect(isSafariId('com.google.Chrome')).toBe(false);
    expect(isSafariId('')).toBe(false);
    expect(isSafariId(null)).toBe(false);
  });
});

describe('one browser per session enforcement', () => {
  it('still allows only one browser per session, naming the one to uncheck', async () => {
    await openAppsStep();
    fireEvent.click(screen.getByLabelText('Google Chrome'));
    fireEvent.click(screen.getByLabelText('Brave'));
    expect(
      screen.getByText(
        'Only one browser per session. Uncheck Google Chrome to use Brave instead.',
      ),
    ).toBeInTheDocument();
    expect(screen.getByLabelText('Brave')).not.toBeChecked();
    expect(screen.getByLabelText('Google Chrome')).toBeChecked();
  });
});
