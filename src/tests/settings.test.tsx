import { beforeEach, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Settings } from '../pages/Settings';
import { useAppStore } from '../state/store';
import { api, emptySnapshot } from '../services/api';
import { settingsAction, preferenceDefaults } from '../services/settings';
import { FirstRunPermissions, PermissionsGuide } from '../components/PermissionsGuide';
vi.mock('../services/api', () => ({
  desktop: true,
  emptySnapshot: {
    sessions: [],
    tasks: [],
    preferences: { theme: 'dark', default_minutes: 25 },
  },
  api: {
    execute: vi.fn(),
    snapshot: vi.fn(),
    focusSetup: vi.fn(),
    requestCamera: vi.fn(),
    openSetup: vi.fn(),
  },
}));
vi.mock('../services/settings', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../services/settings')>()),
  settingsAction: vi.fn(),
}));
beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  HTMLDialogElement.prototype.showModal = function () {
    this.setAttribute('open', '');
  };
  HTMLDialogElement.prototype.close = function () {
    this.removeAttribute('open');
  };
  useAppStore.setState({
    data: structuredClone(emptySnapshot),
    loading: false,
    busy: false,
    error: null,
  });
  vi.mocked(settingsAction).mockResolvedValue({ login: 0, notifications: 0 });
  vi.mocked(api.snapshot).mockResolvedValue(structuredClone(emptySnapshot));
  vi.mocked(api.execute).mockImplementation(async (action) => ({
    ...emptySnapshot,
    preferences:
      action.type === 'update_preferences'
        ? action.preferences
        : preferenceDefaults,
  }));
  vi.mocked(api.focusSetup).mockResolvedValue({
    platform: 'macos',
    native_available: true,
    applications: [],
    camera_permission: 'granted',
    pairing_token: '',
    browser: {
      available: true,
      connected: true,
      applied_id: '',
      error: '',
      blocks: 0,
      cleanup_pending: false,
      extension_stale: false,
    },
    connected_browsers: ['com.brave.Browser'],
  });
});
it('shows the first-run guide once after dismissal', async () => {
  const user = userEvent.setup();
  const view = render(<FirstRunPermissions />);
  expect(await screen.findByRole('dialog')).toHaveTextContent('Permissions & setup');
  await user.click(screen.getByRole('button', { name: 'Done' }));
  view.unmount();
  render(<FirstRunPermissions />);
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
});
it('opens camera settings after denial instead of requesting permission again', async () => {
  const setup = await api.focusSetup();
  vi.mocked(api.focusSetup).mockResolvedValue({ ...setup, camera_permission: 'denied' });
  const user = userEvent.setup();
  render(<PermissionsGuide onClose={() => {}} />);
  await screen.findByText(/Denied — System Settings/);
  await user.click(screen.getAllByRole('button', { name: 'Open settings' })[0]);
  expect(api.openSetup).toHaveBeenCalledWith('camera_settings');
  expect(api.requestCamera).not.toHaveBeenCalled();
});
it('gives Windows permission guidance without Mac-only controls', async () => {
  const setup = await api.focusSetup();
  vi.mocked(api.focusSetup).mockResolvedValue({
    ...setup, platform: 'windows', camera_permission: 'unsupported',
  });
  const user = userEvent.setup();
  render(<PermissionsGuide onClose={() => {}} />);
  expect(await screen.findByRole('button', { name: 'Unavailable' })).toBeDisabled();
  expect(screen.queryByText(/System Settings/)).not.toBeInTheDocument();
  expect(screen.getByText(/More info → Run anyway/)).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Open settings' }));
  expect(api.openSetup).toHaveBeenCalledWith('notification_settings');
});
it('saves real preferences with button choices and no dropdowns', async () => {
  const user = userEvent.setup();
  render(<Settings />);
  expect(screen.queryByRole('combobox')).not.toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Light' }));
  await user.click(
    screen.getByRole('switch', { name: 'Show timer in menu bar' }),
  );
  await user.click(screen.getByRole('button', { name: 'Save changes' }));
  expect(api.execute).toHaveBeenCalledWith(
    expect.objectContaining({
      type: 'update_preferences',
      preferences: expect.objectContaining({
        theme: 'light',
        menu_timer: true,
      }),
    }),
  );
  expect(await screen.findByText('Saved')).toBeInTheDocument();
});
it('uses one connection status and opens a real blocking test', async () => {
  const user = userEvent.setup();
  render(<Settings />);
  await user.click(screen.getByRole('button', { name: 'Connections' }));
  expect(await screen.findAllByText('Connected')).toHaveLength(1);
  expect(screen.getAllByText('Allowed')).toHaveLength(1);
  await user.click(screen.getByRole('button', { name: 'Connected' }));
  await user.click(screen.getByRole('button', { name: 'Test blocking' }));
  expect(settingsAction).toHaveBeenCalledWith({ type: 'test_browser' });
});
it('requires typed confirmation before deleting history', async () => {
  const user = userEvent.setup();
  render(<Settings />);
  await user.click(screen.getByRole('button', { name: 'Data & help' }));
  await user.click(screen.getByRole('button', { name: 'Delete history' }));
  const buttons = screen.getAllByRole('button', { name: 'Delete history' });
  expect(buttons.at(-1)).toBeDisabled();
  await user.type(
    screen.getByRole('textbox', { name: 'Confirmation' }),
    'DELETE HISTORY',
  );
  await user.click(buttons.at(-1)!);
  await waitFor(() =>
    expect(settingsAction).toHaveBeenCalledWith({
      type: 'delete_history',
      confirmation: 'DELETE HISTORY',
    }),
  );
});
it('does not report success when a native action fails', async () => {
  const user = userEvent.setup();
  render(<Settings />);
  await user.click(screen.getByRole('button', { name: 'Data & help' }));
  vi.mocked(settingsAction).mockRejectedValueOnce(new Error('Disk is full.'));
  await user.click(screen.getByRole('button', { name: 'Back up' }));
  expect(await screen.findByRole('status')).toHaveTextContent('Disk is full');
  expect(screen.queryByText('Backup saved.')).not.toBeInTheDocument();
});
