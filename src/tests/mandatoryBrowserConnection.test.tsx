import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { PermissionsGuide } from '../components/PermissionsGuide';
import {
  BrowserSetup,
  areAllInstalledBrowsersConnected,
  getInstalledSupportedBrowsers,
  getUnconnectedInstalledBrowsers,
  isBrowserConnected,
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
      isPrivilegedHelperInstalled: vi.fn().mockResolvedValue(true),
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

  it('allows the Done button in PermissionsGuide without requiring all installed browsers to be connected', async () => {
    vi.mocked(api.focusSetup).mockResolvedValue(multiBrowserSetup);
    const onClose = vi.fn();

    render(<PermissionsGuide onClose={onClose} isFirstRun={true} />);

    // Done button is enabled because connecting all browsers is not compulsory
    const doneBtn = await screen.findByRole('button', { name: 'Done' });
    expect(doneBtn).not.toBeDisabled();
    const user = userEvent.setup();
    await user.click(doneBtn);
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
    const browserDialog = screen.getByRole('dialog', {
      name: 'Browser connection',
    });
    expect(browserDialog).toBeInTheDocument();

    // User can cross the browser connection popup
    const closeDialogBtn = within(browserDialog).getByRole('button', {
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

  it('does NOT show mandatory modal when browsers are unconnected', async () => {
    const openDisconnectedSetup: FocusSetup = {
      ...multiBrowserSetup,
      applications: [
        { id: 'com.google.chrome', name: 'Google Chrome' },
        { id: 'ai.perplexity.comet', name: 'Comet' },
      ],
    };
    vi.mocked(api.focusSetup).mockResolvedValue(openDisconnectedSetup);
    localStorage.setItem('preelock-permissions-v10', 'seen');
    localStorage.setItem('preelock-all-browsers-connected', 'true');

    render(<MandatoryBrowserDisconnectModal />);

    // Mandatory modal must not show
    expect(
      screen.queryByRole('dialog', {
        name: 'Browser connection required',
      }),
    ).not.toBeInTheDocument();
  });

  it('accurately detects disconnected browsers whether open or closed, without falsely marking closed browsers as connected', () => {
    // All browsers disconnected, some open, some closed
    const allDisconnectedSetup: FocusSetup = {
      ...multiBrowserSetup,
      connected_browsers: [],
      browser: {
        ...multiBrowserSetup.browser,
        connected: false,
        client_browser: '',
      },
      applications: [{ id: 'com.brave.Browser', name: 'Brave' }],
    };

    // Brave is open and disconnected -> false
    expect(isBrowserConnected('com.brave.Browser', allDisconnectedSetup)).toBe(
      false,
    );
    // Chrome is closed and disconnected -> false (never falsely marked connected)
    expect(isBrowserConnected('com.google.chrome', allDisconnectedSetup)).toBe(
      false,
    );
    // Comet is closed and disconnected -> false
    expect(
      isBrowserConnected('ai.perplexity.comet', allDisconnectedSetup),
    ).toBe(false);

    // Now close Brave as well
    const allClosedSetup: FocusSetup = {
      ...allDisconnectedSetup,
      applications: [],
    };
    expect(isBrowserConnected('com.brave.Browser', allClosedSetup)).toBe(false);
    expect(isBrowserConnected('com.google.chrome', allClosedSetup)).toBe(false);
    expect(isBrowserConnected('ai.perplexity.comet', allClosedSetup)).toBe(
      false,
    );

    // BrowserSetup renders all as disconnected with Install buttons
    render(
      <BrowserSetup
        setup={allClosedSetup}
        refresh={vi.fn().mockResolvedValue(undefined)}
      />,
    );
    expect(screen.queryByText('✓ Connected')).not.toBeInTheDocument();
    expect(
      screen.getByText('Installed browsers (0/3 connected):'),
    ).toBeInTheDocument();
    const installButtons = screen.getAllByText('Install');
    expect(installButtons).toHaveLength(3);
  });

  it('accurately detects connected browsers whether open or closed, consistently showing them as connected', () => {
    // Brave is paired/connected, but Brave is closed (applications is empty)
    const pairedClosedSetup: FocusSetup = {
      ...multiBrowserSetup,
      connected_browsers: ['com.brave.Browser'],
      browser: {
        ...multiBrowserSetup.browser,
        connected: false,
        client_browser: '',
      },
      applications: [],
    };

    // Paired browser must be detected as connected even when closed
    expect(isBrowserConnected('com.brave.Browser', pairedClosedSetup)).toBe(
      true,
    );
    // Unpaired browsers remain disconnected when closed
    expect(isBrowserConnected('com.google.chrome', pairedClosedSetup)).toBe(
      false,
    );
    expect(isBrowserConnected('ai.perplexity.comet', pairedClosedSetup)).toBe(
      false,
    );

    // BrowserSetup renders Brave as Connected (1/3 connected) even when closed
    render(
      <BrowserSetup
        setup={pairedClosedSetup}
        refresh={vi.fn().mockResolvedValue(undefined)}
      />,
    );
    expect(
      screen.getByText('Installed browsers (1/3 connected):'),
    ).toBeInTheDocument();
    expect(screen.getByText('✓ Connected')).toBeInTheDocument();
  });
});
