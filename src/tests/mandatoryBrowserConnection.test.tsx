import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { PermissionsGuide } from '../components/PermissionsGuide';
import {
  BrowserSetup,
  areAllInstalledBrowsersConnected,
  getInstalledSupportedBrowsers,
  getUnconnectedInstalledBrowsers,
} from '../components/BrowserSetup';
import { MandatoryBrowserDisconnectModal } from '../components/MandatoryBrowserDisconnectModal';
import { api } from '../services/api';
import type { FocusSetup } from '../types';

vi.mock('../services/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../services/api')>();
  return {
    ...actual,
    desktop: true,
    api: {
      ...actual.api,
      focusSetup: vi.fn(),
      openSetup: vi.fn().mockResolvedValue(undefined),
      requestCamera: vi.fn().mockResolvedValue(undefined),
      execute: vi.fn().mockResolvedValue(true),
    },
  };
});

vi.mock('../services/settings', () => ({
  settingsAction: vi.fn().mockResolvedValue({
    accessibility: 1,
    notifications: 2,
    login: 1,
    login_user_pref: true,
  }),
}));

const multiBrowserSetup: FocusSetup = {
  platform: 'macos',
  native_available: true,
  applications: [],
  browsers: [
    { id: 'com.google.chrome', name: 'Google Chrome' },
    { id: 'com.brave.Browser', name: 'Brave' },
    { id: 'ai.perplexity.comet', name: 'Comet' },
    { id: 'com.apple.Safari', name: 'Safari' }, // Safari must be ignored on macOS
  ],
  camera_permission: 'granted',
  accessibility_permission: 'granted',
  browser: {
    available: true,
    connected: false,
    client_browser: '',
    applied_id: '',
    error: '',
    blocks: 0,
    cleanup_pending: false,
    extension_stale: false,
  },
  pairing_token: 'TEST-PAIR-TOKEN-12345',
  connected_browsers: ['com.brave.Browser'], // Only Brave is connected initially
};

describe('Mandatory Browser Connection Requirements', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
  });

  it('getInstalledSupportedBrowsers excludes Safari on macOS', () => {
    const installed = getInstalledSupportedBrowsers(multiBrowserSetup);
    expect(installed.map((b) => b.id)).toEqual([
      'com.google.chrome',
      'com.brave.Browser',
      'ai.perplexity.comet',
    ]);
    expect(installed.some((b) => b.id === 'com.apple.Safari')).toBe(false);
  });

  it('correctly reports unconnected installed browsers and overall connection state', () => {
    const unconnected = getUnconnectedInstalledBrowsers(multiBrowserSetup);
    expect(unconnected.map((b) => b.id)).toEqual([
      'com.google.chrome',
      'ai.perplexity.comet',
    ]);
    expect(areAllInstalledBrowsersConnected(multiBrowserSetup)).toBe(false);

    const allConnectedSetup: FocusSetup = {
      ...multiBrowserSetup,
      connected_browsers: [
        'com.google.chrome',
        'com.brave.Browser',
        'ai.perplexity.comet',
      ],
    };
    expect(areAllInstalledBrowsersConnected(allConnectedSetup)).toBe(true);
    expect(getUnconnectedInstalledBrowsers(allConnectedSetup)).toEqual([]);
  });

  it('disables the Done button in PermissionsGuide until all installed browsers are connected', async () => {
    vi.mocked(api.focusSetup).mockResolvedValue(multiBrowserSetup);
    const onClose = vi.fn();

    const { unmount } = render(
      <PermissionsGuide onClose={onClose} isFirstRun={true} />,
    );

    // Done button is disabled
    const doneBtn = await screen.findByRole('button', { name: 'Done' });
    expect(doneBtn).toBeDisabled();

    // Browser extension row explains which browsers are still needed
    expect(screen.getByText(/Google Chrome, Comet/i)).toBeInTheDocument();
    unmount();

    // Now simulate user having connected all browsers
    const allConnectedSetup: FocusSetup = {
      ...multiBrowserSetup,
      connected_browsers: [
        'com.google.chrome',
        'com.brave.Browser',
        'ai.perplexity.comet',
      ],
    };
    vi.mocked(api.focusSetup).mockResolvedValue(allConnectedSetup);

    render(<PermissionsGuide onClose={onClose} isFirstRun={true} />);

    // Once all browsers connect, Done button is enabled
    const enabledDone = await screen.findByRole('button', { name: 'Done' });
    expect(enabledDone).not.toBeDisabled();
    const user = userEvent.setup();
    await user.click(enabledDone);
    expect(onClose).toHaveBeenCalled();
  });

  it('allows user to open browser connection dialog and cross it', async () => {
    vi.mocked(api.focusSetup).mockResolvedValue(multiBrowserSetup);
    const onClose = vi.fn();
    const user = userEvent.setup();

    render(<PermissionsGuide onClose={onClose} isFirstRun={true} />);

    const connectBtn = await screen.findByRole('button', {
      name: 'Connect browser',
    });
    await user.click(connectBtn);

    // Browser connection modal is opened
    expect(
      screen.getByRole('dialog', { name: 'Browser connection' }),
    ).toBeInTheDocument();

    // User can cross the browser connection popup
    const closeDialogBtn = screen.getByRole('button', {
      name: 'Close dialog',
    });
    await user.click(closeDialogBtn);

    // Browser connection modal closes
    expect(
      screen.queryByRole('dialog', { name: 'Browser connection' }),
    ).not.toBeInTheDocument();
  });

  it('renders all installed browsers with connection status in BrowserSetup', () => {
    render(
      <BrowserSetup
        setup={multiBrowserSetup}
        refresh={vi.fn().mockResolvedValue(undefined)}
      />,
    );

    // Shows connection status for all installed browsers
    expect(screen.getAllByText('Google Chrome').length).toBeGreaterThanOrEqual(
      1,
    );
    expect(screen.getAllByText('Brave').length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('Comet').length).toBeGreaterThanOrEqual(1);

    // Brave is connected
    expect(screen.getByText('✓ Connected')).toBeInTheDocument();

    // Chrome and Comet have install buttons
    const installButtons = screen.getAllByText('Install');
    expect(installButtons).toHaveLength(2);
  });

  it('shows persistent mandatory modal without close button when a browser disconnects in the future', async () => {
    vi.mocked(api.focusSetup).mockResolvedValue(multiBrowserSetup);
    localStorage.setItem('preelock-permissions-v10', 'seen');
    localStorage.setItem('preelock-all-browsers-connected', 'true');

    // multiBrowserSetup has disconnected browsers (Chrome and Comet)
    const { unmount } = render(<MandatoryBrowserDisconnectModal />);

    // The modal appears
    expect(
      await screen.findByRole('dialog', {
        name: 'Browser connection required',
      }),
    ).toBeInTheDocument();

    // It has NO close button
    expect(screen.queryByLabelText('Close dialog')).not.toBeInTheDocument();

    // It warns about disconnected browsers
    expect(screen.getByText(/Google Chrome, Comet/i)).toBeInTheDocument();
    unmount();

    // Once reconnected, the modal automatically closes
    const allConnectedSetup: FocusSetup = {
      ...multiBrowserSetup,
      connected_browsers: [
        'com.google.chrome',
        'com.brave.Browser',
        'ai.perplexity.comet',
      ],
    };
    vi.mocked(api.focusSetup).mockResolvedValue(allConnectedSetup);

    render(<MandatoryBrowserDisconnectModal />);
    expect(
      screen.queryByRole('dialog', { name: 'Browser connection required' }),
    ).not.toBeInTheDocument();
  });
});
