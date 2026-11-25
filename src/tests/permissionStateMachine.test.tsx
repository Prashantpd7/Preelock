import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {
  FirstRunPermissions,
  PermissionsGuide,
} from '../components/PermissionsGuide';
import { api } from '../services/api';
import { settingsAction } from '../services/settings';
import { useAppStore } from '../state/store';
import type { FocusSetup, Snapshot } from '../types';

const baseSnapshot: Snapshot = {
  sessions: [],
  tasks: [],
  schedules: [],
  one_shots: [],
  preferences: {
    theme: 'dark',
    default_minutes: 25,
    onboarding_completed: false,
  },
};

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
      isPrivilegedHelperInstalled: vi.fn().mockResolvedValue(true),
      execute: vi.fn().mockImplementation(async (action) => {
        if (action?.type === 'update_preferences' && action?.preferences) {
          return {
            ...baseSnapshot,
            preferences: { ...baseSnapshot.preferences, ...action.preferences },
          };
        }
        return structuredClone(baseSnapshot);
      }),
    },
  };
});

vi.mock('../services/settings', () => ({
  settingsAction: vi.fn(),
}));

const baseSetup: FocusSetup = {
  platform: 'macos',
  native_available: true,
  applications: [],
  camera_permission: 'granted',
  accessibility_permission: 'not_granted',
  pairing_token: 'PAIR-TOKEN-TEST',
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
};

describe('Accessibility and Onboarding Permission State Machine', () => {
  beforeEach(() => {
    localStorage.clear();
    vi.clearAllMocks();
    useAppStore.setState({ data: structuredClone(baseSnapshot) });
    vi.mocked(api.focusSetup).mockResolvedValue(structuredClone(baseSetup));
    vi.mocked(settingsAction).mockResolvedValue({
      accessibility: 0,
      accessibility_permission: 'not_granted',
      notifications: 2,
    });
  });

  afterEach(() => {
    useAppStore.setState({ data: structuredClone(baseSnapshot) });
  });

  it('Requirement 5.3: auto-dismisses when all capabilities are already granted on first launch', async () => {
    vi.mocked(api.focusSetup).mockResolvedValue({
      ...baseSetup,
      accessibility_permission: 'granted',
      connected_browsers: ['com.brave.Browser'],
      browser: {
        ...baseSetup.browser,
        connected: true,
        applied_id: 'com.brave.Browser',
      },
    });

    render(<FirstRunPermissions />);

    // Should auto-dismiss the modal once permissions are checked and confirmed
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });

    // Should mark onboarding completed in backend preferences
    await waitFor(() => {
      expect(api.execute).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'update_preferences',
          preferences: expect.objectContaining({ onboarding_completed: true }),
        }),
      );
    });
    expect(localStorage.getItem('preelock-permissions-v10')).toBe('seen');
  });

  it('Requirement 1 & 6: never interrupts an active strict or preparing session', () => {
    useAppStore.setState({
      data: {
        ...baseSnapshot,
        sessions: [
          {
            id: 'active-session-1',
            title: 'Deep Work',
            duration_minutes: 45,
            remaining_ms: 1800000,
            focused_ms: 600000,
            allow_pause: false,
            status: 'running',
            started_at: Date.now() - 600000,
            ended_at: null,
            deadline_ms: null,
          },
        ],
      },
    });

    render(<FirstRunPermissions />);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('does not display onboarding when preferences.onboarding_completed is already true', () => {
    useAppStore.setState({
      data: {
        ...baseSnapshot,
        preferences: {
          ...baseSnapshot.preferences,
          onboarding_completed: true,
        },
      },
    });

    render(<FirstRunPermissions />);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('PermissionsGuide displays clear explanation and Open settings when Accessibility is missing', async () => {
    render(<PermissionsGuide onClose={vi.fn()} />);

    expect(
      await screen.findByRole('heading', { name: 'Permissions & setup' }),
    ).toBeInTheDocument();

    // Accessibility row should show Open settings
    const openSettingsBtn = await screen.findByRole('button', {
      name: 'Open settings',
    });
    expect(openSettingsBtn).toBeInTheDocument();
    expect(openSettingsBtn).not.toBeDisabled();
    expect(
      screen.getByText(
        /Required for Screen Time activity tracking and distraction blocking/i,
      ),
    ).toBeInTheDocument();
  });

  it('clicking Open settings triggers accessibility_settings request', async () => {
    const user = userEvent.setup();
    render(<PermissionsGuide onClose={vi.fn()} />);

    const openSettingsBtn = await screen.findByRole('button', {
      name: 'Open settings',
    });
    await user.click(openSettingsBtn);

    expect(api.openSetup).toHaveBeenCalledWith('accessibility_settings');
  });

  it('dynamically transitions to Allowed when permission is recognized', async () => {
    vi.mocked(api.focusSetup).mockResolvedValue({
      ...baseSetup,
      accessibility_permission: 'granted',
    });
    vi.mocked(settingsAction).mockResolvedValue({
      accessibility: 1,
      accessibility_permission: 'granted',
      notifications: 2,
    });

    render(<PermissionsGuide onClose={vi.fn()} />);

    await waitFor(() => {
      const accessibilityRow = screen
        .getByText('Accessibility')
        .closest('.preference-row');
      expect(accessibilityRow).not.toBeNull();
      const allowedBtn = within(accessibilityRow as HTMLElement).getByRole(
        'button',
        { name: 'Allowed' },
      );
      expect(allowedBtn).toBeInTheDocument();
      expect(allowedBtn).toBeDisabled();
    });
    expect(
      screen.getByText(
        /Screen Time activity tracking and distraction blocking are active/i,
      ),
    ).toBeInTheDocument();
  });

  it('refreshes permission state via recheck_permissions on window focus event', async () => {
    render(<PermissionsGuide onClose={vi.fn()} />);

    await screen.findByRole('heading', { name: 'Permissions & setup' });

    // Simulate window focus (user switching back from System Settings)
    window.dispatchEvent(new Event('focus'));

    await waitFor(() => {
      expect(settingsAction).toHaveBeenCalledWith({
        type: 'recheck_permissions',
      });
    });
  });

  it('Requirement 8: on Windows, omits macOS Accessibility row since UI Automation needs no user prompt', async () => {
    vi.mocked(api.focusSetup).mockResolvedValue({
      ...baseSetup,
      platform: 'windows',
    });

    render(<PermissionsGuide onClose={vi.fn()} />);

    await screen.findByRole('heading', { name: 'Permissions & setup' });

    // Accessibility row should not exist on Windows
    expect(screen.queryByText('Accessibility')).not.toBeInTheDocument();
  });

  it('migrates legacy localStorage-only users by recognizing localStorage seen flag even when backend preference was false', () => {
    localStorage.setItem('preelock-permissions-v10', 'seen');
    useAppStore.setState({
      data: {
        ...baseSnapshot,
        preferences: {
          ...baseSnapshot.preferences,
          onboarding_completed: false,
        },
      },
    });

    render(<FirstRunPermissions />);

    // Modal should not render
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('reflects permission revocation dynamically in the UI', async () => {
    // Start with granted
    vi.mocked(api.focusSetup).mockResolvedValue({
      ...baseSetup,
      accessibility_permission: 'granted',
    });
    vi.mocked(settingsAction).mockResolvedValue({
      accessibility: 1,
      accessibility_permission: 'granted',
      notifications: 2,
    });

    const { rerender } = render(<PermissionsGuide onClose={vi.fn()} />);

    await waitFor(() => {
      const accessibilityRow = screen
        .getByText('Accessibility')
        .closest('.preference-row');
      expect(
        within(accessibilityRow as HTMLElement).getByRole('button', {
          name: 'Allowed',
        }),
      ).toBeInTheDocument();
    });

    // Revoke permission in OS
    vi.mocked(api.focusSetup).mockResolvedValue({
      ...baseSetup,
      accessibility_permission: 'not_granted',
    });
    vi.mocked(settingsAction).mockResolvedValue({
      accessibility: 0,
      accessibility_permission: 'not_granted',
      notifications: 2,
    });

    // Simulate window focus recheck
    window.dispatchEvent(new Event('focus'));

    rerender(<PermissionsGuide onClose={vi.fn()} />);

    await waitFor(() => {
      const accessibilityRow = screen
        .getByText('Accessibility')
        .closest('.preference-row');
      const openBtn = within(accessibilityRow as HTMLElement).getByRole(
        'button',
        { name: 'Open settings' },
      );
      expect(openBtn).toBeInTheDocument();
      expect(openBtn).not.toBeDisabled();
    });
  });
});
