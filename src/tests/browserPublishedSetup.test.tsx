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

    // Check status
    expect(screen.getByText('Browser is not connected.')).toBeInTheDocument();
    expect(
      screen.getByText('Step 1: Select and install a browser extension'),
    ).toBeInTheDocument();
    expect(
      screen.getByText('Step 2: Connect your browser'),
    ).toBeInTheDocument();

    // Default browser is Brave in mockSetup -> dropdown selects Brave, button is "Add to Brave"
    const select = screen.getByLabelText('Select browser') as HTMLSelectElement;
    expect(select.value).toBe('com.brave.Browser');
    expect(
      screen.getByRole('button', { name: 'Add to Brave' }),
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

  it('dynamically changes button text and launches selected browser with official listing', async () => {
    const user = userEvent.setup();
    render(<BrowserSetup setup={mockSetup} refresh={vi.fn()} />);

    const select = screen.getByLabelText('Select browser');
    const installBtn = screen.getByRole('button', { name: 'Add to Brave' });

    // Click install button for Brave
    await user.click(installBtn);
    expect(api.openSetup).toHaveBeenCalledWith(
      'install_extension',
      'com.brave.Browser',
    );

    // Change dropdown to Chrome
    await user.selectOptions(select, 'com.google.chrome');
    const chromeBtn = screen.getByRole('button', { name: 'Add to Chrome' });
    expect(chromeBtn).toBeInTheDocument();

    await user.click(chromeBtn);
    expect(api.openSetup).toHaveBeenCalledWith(
      'install_extension',
      'com.google.chrome',
    );

    // Change dropdown to Microsoft Edge
    await user.selectOptions(select, 'com.microsoft.edgemac');
    const edgeBtn = screen.getByRole('button', { name: 'Add to Edge' });
    expect(edgeBtn).toBeInTheDocument();

    await user.click(edgeBtn);
    expect(api.openSetup).toHaveBeenCalledWith(
      'install_extension',
      'com.microsoft.edgemac',
    );

    // Change dropdown to Comet
    await user.selectOptions(select, 'ai.perplexity.comet');
    const cometBtn = screen.getByRole('button', { name: 'Add to Comet' });
    expect(cometBtn).toBeInTheDocument();

    await user.click(cometBtn);
    expect(api.openSetup).toHaveBeenCalledWith(
      'install_extension',
      'ai.perplexity.comet',
    );
  });

  it('handles unsupported default browser by falling back to first supported browser', () => {
    const setupWithSafariDefault: FocusSetup = {
      ...mockSetup,
      default_browser: 'com.apple.Safari',
      browsers: [
        { id: 'com.google.chrome', name: 'Google Chrome' },
        { id: 'com.brave.Browser', name: 'Brave' },
      ],
    };

    render(<BrowserSetup setup={setupWithSafariDefault} refresh={vi.fn()} />);

    const select = screen.getByLabelText('Select browser') as HTMLSelectElement;
    // Safari is unsupported and excluded; falls back to Chrome
    expect(select.value).toBe('com.google.chrome');
    expect(
      screen.getByRole('button', { name: 'Add to Chrome' }),
    ).toBeInTheDocument();
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
