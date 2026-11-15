import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { BrowserSetup } from '../components/BrowserSetup';
import {
  PermissionsGuide,
  FirstRunPermissions,
} from '../components/PermissionsGuide';
import { api } from '../services/api';
import type { FocusSetup } from '../types';

vi.mock('../services/api', () => ({
  desktop: true,
  emptySnapshot: {
    sessions: [],
    tasks: [],
    preferences: { theme: 'dark', default_minutes: 25 },
  },
  api: {
    focusSetup: vi.fn(),
    resetPairing: vi.fn(),
    openSetup: vi.fn(),
    requestCamera: vi.fn(),
  },
}));

vi.mock('../services/settings', () => ({
  settingsAction: vi.fn().mockResolvedValue({}),
}));

const mockSetup: FocusSetup = {
  platform: 'macos',
  native_available: true,
  applications: [],
  camera_permission: 'granted',
  pairing_token: 'PAIR-TOKEN-TEST-1234567890ABCDEF1234567890ABCDEF',
  browser: {
    available: true,
    connected: false,
    applied_id: '',
    error: '',
    blocks: 0,
    cleanup_pending: false,
    extension_stale: false,
  },
  connected_browsers: [],
  browsers: [
    { id: 'com.google.chrome', name: 'Google Chrome' },
    { id: 'com.brave.Browser', name: 'Brave' },
    { id: 'com.microsoft.edgemac', name: 'Microsoft Edge' },
    { id: 'ai.perplexity.comet', name: 'Comet' },
    { id: 'com.vivaldi.Vivaldi', name: 'Vivaldi' },
  ],
  default_browser: 'com.brave.Browser',
};

describe('Published Browser Extension Integration & UI', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders Step 1 and Step 2 with auto-selected default browser and dynamic button', async () => {
    render(<BrowserSetup setup={mockSetup} refresh={vi.fn()} />);

    // Step 1 directly shows installed browsers without duplicate warning
    expect(
      screen.getByText('Step 1: Select and install a browser extension'),
    ).toBeInTheDocument();
    expect(screen.getByText(/Installed browsers/)).toBeInTheDocument();
    expect(screen.getByText('Brave')).toBeInTheDocument();
    expect(screen.getByText('Google Chrome')).toBeInTheDocument();
    expect(screen.getByText('Microsoft Edge')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Add to Brave' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Add to Chrome' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Add to Edge' }),
    ).toBeInTheDocument();

    expect(
      screen.getByText('Step 2: Connect your browser'),
    ).toBeInTheDocument();

    // Connection code and copy button
    expect(screen.getByLabelText('Connection code')).toHaveValue(
      'PAIR-TOKEN-TEST-1234567890ABCDEF1234567890ABCDEF',
    );
    expect(
      screen.getByRole('button', { name: 'Copy code' }),
    ).toBeInTheDocument();

    // Text below code strictly matches requirement
    expect(
      screen.getByText(
        'Copy the code and paste it in the Preelock extension in your browser.',
      ),
    ).toBeInTheDocument();

    // Verifies absence of old manual instructions
    expect(screen.queryByText(/Developer mode/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/Load unpacked/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/⌘⇧G/i)).not.toBeInTheDocument();
    expect(
      screen.queryByText(/Extension not installed/i),
    ).not.toBeInTheDocument();
  });

  it('dynamically launches selected browser with official listing', async () => {
    const user = userEvent.setup();
    render(<BrowserSetup setup={mockSetup} refresh={vi.fn()} />);

    // Click install button for Brave
    const braveBtn = screen.getByRole('button', { name: 'Add to Brave' });
    await user.click(braveBtn);
    expect(api.openSetup).toHaveBeenCalledWith(
      'install_extension',
      'com.brave.Browser',
    );

    // Click install button for Chrome
    const chromeBtn = screen.getByRole('button', { name: 'Add to Chrome' });
    await user.click(chromeBtn);
    expect(api.openSetup).toHaveBeenCalledWith(
      'install_extension',
      'com.google.chrome',
    );

    // Click install button for Microsoft Edge
    const edgeBtn = screen.getByRole('button', { name: 'Add to Edge' });
    await user.click(edgeBtn);
    expect(api.openSetup).toHaveBeenCalledWith(
      'install_extension',
      'com.microsoft.edgemac',
    );
  });

  it('handles unsupported default browser by excluding it from installed list', () => {
    const setupWithSafariDefault: FocusSetup = {
      ...mockSetup,
      default_browser: 'com.apple.Safari',
      browsers: [
        { id: 'com.google.chrome', name: 'Google Chrome' },
        { id: 'com.brave.Browser', name: 'Brave' },
      ],
    };

    render(<BrowserSetup setup={setupWithSafariDefault} refresh={vi.fn()} />);

    // Safari is unsupported and excluded; shows Chrome and Brave in installed list
    expect(screen.getByText('Google Chrome')).toBeInTheDocument();
    expect(screen.getByText('Brave')).toBeInTheDocument();
    expect(screen.queryByText('Safari')).not.toBeInTheDocument();
  });

  it('handles zero installed browsers gracefully', () => {
    const setupNoBrowsers: FocusSetup = {
      ...mockSetup,
      browsers: [],
      default_browser: undefined,
    };

    render(<BrowserSetup setup={setupNoBrowsers} refresh={vi.fn()} />);

    expect(
      screen.getByText(
        'No supported browser detected. Please install Google Chrome, Brave, Microsoft Edge, or Comet.',
      ),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText('Select browser')).not.toBeInTheDocument();
  });

  it('copies connection code and gives user feedback', async () => {
    const user = userEvent.setup();
    const writeTextMock = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText: writeTextMock },
      configurable: true,
      writable: true,
    });

    render(<BrowserSetup setup={mockSetup} refresh={vi.fn()} />);

    const copyBtn = screen.getByRole('button', { name: 'Copy code' });
    await user.click(copyBtn);

    expect(writeTextMock).toHaveBeenCalledWith(
      'PAIR-TOKEN-TEST-1234567890ABCDEF1234567890ABCDEF',
    );
    expect(screen.getByRole('button', { name: 'Copied' })).toBeInTheDocument();
  });

  it('allows adding another browser and renders compact layout without old paragraph', async () => {
    const user = userEvent.setup();
    const onBackMock = vi.fn();
    render(
      <BrowserSetup
        setup={{
          ...mockSetup,
          connected_browsers: ['ai.perplexity.comet'],
        }}
        refresh={vi.fn()}
        allowAdd={true}
        onBack={onBackMock}
      />,
    );

    // Has back button
    const backBtn = screen.getByRole('button', {
      name: '← Back to connected browsers',
    });
    expect(backBtn).toBeInTheDocument();
    await user.click(backBtn);
    expect(onBackMock).toHaveBeenCalled();

    // Does NOT contain the old paragraph description
    expect(
      screen.queryByText(/Pair an additional browser/i),
    ).not.toBeInTheDocument();

    // Contains the dropdown and Add to Browser button
    expect(screen.getByLabelText('Select browser')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Add to Brave' }),
    ).toBeInTheDocument();

    // Contains Connection code field and strict text
    expect(screen.getByLabelText('Connection code')).toHaveValue(
      'PAIR-TOKEN-TEST-1234567890ABCDEF1234567890ABCDEF',
    );
    expect(
      screen.getByText(
        'Copy the code and paste it in the Preelock extension in your browser.',
      ),
    ).toBeInTheDocument();
  });

  it('renders unified new connection flow inside PermissionsGuide / onboarding', async () => {
    vi.mocked(api.focusSetup).mockResolvedValue(mockSetup);
    const user = userEvent.setup();

    render(<PermissionsGuide onClose={vi.fn()} />);

    // Click "Connect browser" row
    const connectBtn = await screen.findByRole('button', {
      name: 'Connect browser',
    });
    await user.click(connectBtn);

    // Modal opens with unified new experience: Step 1, Step 2, Add to Brave
    expect(
      await screen.findByText('Step 1: Select and install a browser extension'),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Add to Brave' }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText('Connection code')).toHaveValue(
      'PAIR-TOKEN-TEST-1234567890ABCDEF1234567890ABCDEF',
    );

    // No old manual guide
    expect(screen.queryByText(/Load unpacked/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/Developer mode/i)).not.toBeInTheDocument();
  });

  it('FirstRunPermissions displays the permissions guide for first-time launch', async () => {
    localStorage.clear();
    vi.mocked(api.focusSetup).mockResolvedValue(mockSetup);

    render(<FirstRunPermissions />);

    expect(
      await screen.findByRole('heading', { name: 'Permissions & setup' }),
    ).toBeInTheDocument();
    expect(screen.getByText('Browser extension')).toBeInTheDocument();
  });
});
