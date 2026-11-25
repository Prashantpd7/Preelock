import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { InstallHelperModal } from '../components/InstallHelperModal';
import { HelperReminderPrompt } from '../components/HelperReminderPrompt';
import { PermissionsGuide } from '../components/PermissionsGuide';
import { api } from '../services/api';

vi.mock('../services/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../services/api')>();
  return {
    ...actual,
    desktop: true,
    api: {
      ...actual.api,
      focusSetup: vi.fn(),
      openSetup: vi.fn(),
      requestCamera: vi.fn(),
      isPrivilegedHelperInstalled: vi.fn(),
      installPrivilegedHelper: vi.fn(),
      setupNativeBlocking: vi.fn(),
      execute: vi.fn(),
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

describe('Privileged Blocking Helper Enforcement', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('InstallHelperModal renders with Cancel Session and Install Helper buttons', async () => {
    const onCancel = vi.fn();
    const onInstalled = vi.fn();
    const onClose = vi.fn();

    render(
      <InstallHelperModal
        isOpen={true}
        onClose={onClose}
        onCancel={onCancel}
        onInstalled={onInstalled}
        platform="macos"
      />,
    );

    expect(
      screen.getByText('Privileged Blocking Helper Required'),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Cancel Session' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Install Helper' }),
    ).toBeInTheDocument();

    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Cancel Session' }));
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onInstalled).not.toHaveBeenCalled();
  });

  it('InstallHelperModal triggers installPrivilegedHelper and calls onInstalled on success', async () => {
    vi.mocked(api.installPrivilegedHelper).mockResolvedValue(undefined);
    vi.mocked(api.isPrivilegedHelperInstalled).mockResolvedValue(true);

    const onCancel = vi.fn();
    const onInstalled = vi.fn();
    const onClose = vi.fn();

    render(
      <InstallHelperModal
        isOpen={true}
        onClose={onClose}
        onCancel={onCancel}
        onInstalled={onInstalled}
        platform="macos"
      />,
    );

    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Install Helper' }));

    await waitFor(() => {
      expect(api.installPrivilegedHelper).toHaveBeenCalledTimes(1);
      expect(onInstalled).toHaveBeenCalledTimes(1);
    });
  });

  it('InstallHelperModal does not proceed and shows error if user cancels install dialog', async () => {
    vi.mocked(api.installPrivilegedHelper).mockResolvedValue(undefined);
    vi.mocked(api.isPrivilegedHelperInstalled).mockResolvedValue(false);

    const onCancel = vi.fn();
    const onInstalled = vi.fn();
    const onClose = vi.fn();

    render(
      <InstallHelperModal
        isOpen={true}
        onClose={onClose}
        onCancel={onCancel}
        onInstalled={onInstalled}
        platform="macos"
      />,
    );

    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Install Helper' }));

    await waitFor(() => {
      expect(api.installPrivilegedHelper).toHaveBeenCalledTimes(1);
      expect(onInstalled).not.toHaveBeenCalled();
      expect(
        screen.getByText(/Installation was cancelled or not completed/i),
      ).toBeInTheDocument();
    });
  });

  it('PermissionsGuide has helper row and disables Done button until helper is installed', async () => {
    vi.mocked(api.focusSetup).mockResolvedValue({
      platform: 'macos',
      native_available: true,
      applications: [],
      camera_permission: 'granted',
      accessibility_permission: 'granted',
      pairing_token: 'TOKEN',
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
      browsers: [{ id: 'com.brave.Browser', name: 'Brave' }],
    });
    vi.mocked(api.isPrivilegedHelperInstalled).mockResolvedValue(false);

    const onClose = vi.fn();
    render(<PermissionsGuide onClose={onClose} />);

    // Helper row is displayed
    expect(
      await screen.findByText('Privileged Blocking Helper'),
    ).toBeInTheDocument();

    // Done button must be disabled when helper is not installed
    const doneBtn = screen.getByRole('button', { name: 'Done' });
    expect(doneBtn).toBeDisabled();

    // Now user clicks Install Helper
    vi.mocked(api.installPrivilegedHelper).mockResolvedValue(undefined);
    vi.mocked(api.isPrivilegedHelperInstalled).mockResolvedValue(true);

    const installBtn = screen.getByRole('button', { name: 'Install Helper' });
    const user = userEvent.setup();
    await user.click(installBtn);

    // After installation, Done button becomes enabled
    await waitFor(() => {
      expect(doneBtn).not.toBeDisabled();
    });

    await user.click(doneBtn);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('HelperReminderPrompt triggers helper check and handles dismissal', async () => {
    vi.mocked(api.isPrivilegedHelperInstalled).mockResolvedValue(false);

    render(<HelperReminderPrompt initialDelayMs={0} />);

    expect(
      await screen.findByText(
        'Install helper to detect and block websites across all apps.',
      ),
    ).toBeInTheDocument();

    const cancelBtn = screen.getByRole('button', { name: 'Cancel' });
    const user = userEvent.setup();
    await user.click(cancelBtn);

    await waitFor(() => {
      expect(
        screen.queryByText(
          'Install helper to detect and block websites across all apps.',
        ),
      ).not.toBeInTheDocument();
    });
  });
});
