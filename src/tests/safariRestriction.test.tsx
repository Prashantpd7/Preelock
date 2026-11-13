import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
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

describe('Safari is refused for session website blocking', () => {
  it('shows the compatibility notice with real browser connection status', async () => {
    await openAppsStep();
    enableWebsiteBlocking();
    selectSafari();
    expect(
      await screen.findByText("Safari isn't supported yet"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        'Preelock needs a supported browser to block websites during your session.',
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Google Chrome · Connected' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Brave · Not connected' }),
    ).toBeInTheDocument();
  });

  it('applies to scheduled and one-shot sessions too', async () => {
    await openAppsStep(/New schedule/);
    enableWebsiteBlocking();
    selectSafari();
    expect(
      await screen.findByText("Safari isn't supported yet"),
    ).toBeInTheDocument();
  });

  it('swaps Safari for a connected browser and keeps other selections', async () => {
    await openAppsStep();
    fireEvent.click(screen.getByLabelText('Visual Studio Code'));
    enableWebsiteBlocking();
    selectSafari();
    fireEvent.click(
      await screen.findByRole('button', { name: 'Google Chrome · Connected' }),
    );
    expect(
      screen.queryByText("Safari isn't supported yet"),
    ).not.toBeInTheDocument();
    expect(screen.getByLabelText('Safari')).not.toBeChecked();
    expect(screen.getByLabelText('Google Chrome')).toBeChecked();
    // Every other selected application survives the swap.
    expect(screen.getByLabelText('Visual Studio Code')).toBeChecked();
  });

  it('opens the existing Connect browser guide for an unconnected browser', async () => {
    await openAppsStep();
    enableWebsiteBlocking();
    selectSafari();
    fireEvent.click(
      await screen.findByRole('button', { name: 'Brave · Not connected' }),
    );
    expect(
      screen.queryByText("Safari isn't supported yet"),
    ).not.toBeInTheDocument();
    // The unchanged connection flow: same guide, same pairing code.
    expect(
      await screen.findByText(/Browser is not connected/),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: /Add to Brave/ }),
    ).toBeInTheDocument();
    expect(mocked.focusSetup).toHaveBeenCalledWith('com.brave.Browser');
  });

  it('offers official download links when no supported browser is installed', async () => {
    mocked.focusSetup.mockImplementation(async () =>
      setupFixture({ browsers: [], connected_browsers: [] }),
    );
    await openAppsStep();
    enableWebsiteBlocking();
    selectSafari();
    expect(
      await screen.findByText('No supported browser found'),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Brave' }));
    expect(mocked.openSetup).toHaveBeenCalledWith(
      'browser_download',
      'com.brave.Browser',
    );
    fireEvent.click(screen.getByRole('button', { name: 'Get a browser' }));
    expect(mocked.openSetup).toHaveBeenCalledWith(
      'browser_download',
      'com.google.Chrome',
    );
  });

  it('deselects Safari when the notice is cancelled', async () => {
    await openAppsStep();
    enableWebsiteBlocking();
    selectSafari();
    await screen.findByText("Safari isn't supported yet");
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(
      screen.queryByText("Safari isn't supported yet"),
    ).not.toBeInTheDocument();
    expect(screen.getByLabelText('Safari')).not.toBeChecked();
  });

  it('never appears for a session that blocks no websites', async () => {
    await openAppsStep();
    selectSafari();
    await Promise.resolve();
    expect(
      screen.queryByText("Safari isn't supported yet"),
    ).not.toBeInTheDocument();
    // Safari stays a normal selectable app for such a session.
    expect(screen.getByLabelText('Safari')).toBeChecked();
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

describe('choosing a browser opens it and keeps it manageable', () => {
  // Brave installed but not running: it is in the browser list (with its
  // connection status) but not yet in the application list the picker
  // renders. This is the reported stuck state.
  const installedOnly = (over: Partial<FocusSetup> = {}) =>
    setupFixture({
      applications: [
        { id: 'com.apple.Safari', name: 'Safari' },
        { id: 'com.google.Chrome', name: 'Google Chrome' },
        { id: 'com.microsoft.VSCode', name: 'Visual Studio Code' },
      ],
      ...over,
    });

  it('launches the chosen browser and reveals it in Apps & websites', async () => {
    let launched = false;
    mocked.openSetup.mockImplementation(async () => {
      launched = true;
    });
    mocked.focusSetup.mockImplementation(async () =>
      launched
        ? setupFixture({ browser: notConnected, connected_browsers: [] })
        : installedOnly({ browser: notConnected, connected_browsers: [] }),
    );
    await openAppsStep();
    enableWebsiteBlocking();
    selectSafari();
    fireEvent.click(
      await screen.findByRole('button', { name: 'Brave · Not connected' }),
    );
    // The installed browser is opened through the desktop app.
    expect(mocked.openSetup).toHaveBeenCalledWith(
      'browser',
      'com.brave.Browser',
    );
    // It appears in the picker without pressing "Refresh apps"…
    expect(await screen.findByLabelText('Brave')).toBeChecked();
    // …and its extension connection is still what decides the guide.
    expect(
      await screen.findByText(/Browser is not connected/),
    ).toBeInTheDocument();
  });

  it('checks the extension connection independently of the launch', async () => {
    await openAppsStep();
    enableWebsiteBlocking();
    selectSafari();
    fireEvent.click(
      await screen.findByRole('button', { name: 'Google Chrome · Connected' }),
    );
    expect(mocked.openSetup).toHaveBeenCalledWith(
      'browser',
      'com.google.Chrome',
    );
    expect(screen.getByLabelText('Google Chrome')).toBeChecked();
    // A connected browser needs no setup guide even though it was opened.
    expect(
      screen.queryByText(/Browser is not connected/),
    ).not.toBeInTheDocument();
  });

  it('keeps the selection usable when the browser cannot be launched', async () => {
    mocked.openSetup.mockRejectedValue(new Error('launch failed'));
    mocked.focusSetup.mockImplementation(async () =>
      installedOnly({ connected_browsers: [] }),
    );
    await openAppsStep();
    enableWebsiteBlocking();
    selectSafari();
    fireEvent.click(
      await screen.findByRole('button', { name: 'Brave · Not connected' }),
    );
    expect(
      await screen.findByText(
        "Couldn't open browser. Open the browser manually and refresh the app list.",
      ),
    ).toBeInTheDocument();
    // It is still listed and checked, so the user can uncheck it again
    // instead of being stuck with a browser the list never shows.
    const box = await screen.findByLabelText('Brave');
    expect(box).toBeChecked();
    fireEvent.click(box);
    expect(screen.queryByLabelText('Brave')).not.toBeInTheDocument();
  });

  it('switches browsers without keeping the previous one selected', async () => {
    await openAppsStep();
    fireEvent.click(screen.getByLabelText('Visual Studio Code'));
    fireEvent.click(screen.getByLabelText('Google Chrome'));
    enableWebsiteBlocking();
    selectSafari();
    fireEvent.click(
      await screen.findByRole('button', { name: 'Brave · Not connected' }),
    );
    await waitFor(() =>
      expect(screen.getByLabelText('Google Chrome')).not.toBeChecked(),
    );
    expect(screen.getByLabelText('Brave')).toBeChecked();
    // Non-browser selections survive the switch untouched.
    expect(screen.getByLabelText('Visual Studio Code')).toBeChecked();
  });

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
