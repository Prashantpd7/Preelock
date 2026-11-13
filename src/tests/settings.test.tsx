import { beforeEach, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Settings } from '../pages/Settings';
import { useAppStore } from '../state/store';
import { api, emptySnapshot } from '../services/api';
import { settingsAction, preferenceDefaults } from '../services/settings';
import {
  FirstRunPermissions,
  PermissionsGuide,
} from '../components/PermissionsGuide';
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
  vi.mocked(settingsAction).mockResolvedValue({
    login: 0,
    login_user_pref: false,
    notifications: 0,
  });
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
  expect(await screen.findByRole('dialog')).toHaveTextContent(
    'Permissions & setup',
  );
  await user.click(screen.getByRole('button', { name: 'Done' }));
  view.unmount();
  render(<FirstRunPermissions />);
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
});
it('opens camera settings after denial instead of requesting permission again', async () => {
  const setup = await api.focusSetup();
  vi.mocked(api.focusSetup).mockResolvedValue({
    ...setup,
    camera_permission: 'denied',
  });
  const user = userEvent.setup();
  render(<PermissionsGuide onClose={() => {}} />);
  await screen.findByText(/Denied — System Settings/);
  await user.click(screen.getAllByRole('button', { name: 'Open settings' })[0]);
  expect(api.openSetup).toHaveBeenCalledWith('camera_settings');
  expect(api.requestCamera).not.toHaveBeenCalled();
  expect(await screen.findByRole('status')).toHaveTextContent(
    'Turn on Preelock under Camera, then return here.',
  );
});
it('requests camera permission from the guide without starting a session', async () => {
  const setup = await api.focusSetup();
  vi.mocked(api.focusSetup).mockResolvedValue({
    ...setup,
    camera_permission: 'not_requested',
  });
  const user = userEvent.setup();
  render(<PermissionsGuide onClose={() => {}} />);
  const button = await screen.findByRole('button', { name: 'Allow camera' });
  expect(button).toBeEnabled();
  await user.click(button);
  expect(api.requestCamera).toHaveBeenCalledTimes(1);
  expect(api.openSetup).not.toHaveBeenCalled();
  expect(api.execute).not.toHaveBeenCalled();
  expect(await screen.findByRole('status')).toHaveTextContent(
    'Choose Allow in the Mac permission window.',
  );
});
it('gives Windows permission guidance without Mac-only controls', async () => {
  const setup = await api.focusSetup();
  vi.mocked(api.focusSetup).mockResolvedValue({
    ...setup,
    platform: 'windows',
    camera_permission: 'unsupported',
  });
  const user = userEvent.setup();
  render(<PermissionsGuide onClose={() => {}} />);
  expect(
    await screen.findByRole('button', { name: 'Unavailable' }),
  ).toBeDisabled();
  expect(screen.queryByText(/System Settings/)).not.toBeInTheDocument();
  expect(screen.getByText(/More info → Run anyway/)).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Open settings' }));
  expect(api.openSetup).toHaveBeenCalledWith('notification_settings');
});
it('saves real preferences with button choices and no dropdowns', async () => {
  const user = userEvent.setup();
  render(<Settings />);
  expect(screen.queryByRole('combobox')).not.toBeInTheDocument();
  // Item 3: the menu-bar timer is ON by default for a fresh profile.
  expect(
    screen.getByRole('switch', { name: 'Show timer in menu bar' }),
  ).toBeChecked();
  await user.click(screen.getByRole('button', { name: 'Light' }));
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
it('defaults the menu-bar timer and session sounds on', async () => {
  const user = userEvent.setup();
  // Item 3: fresh defaults are all on.
  expect(preferenceDefaults.menu_timer).toBe(true);
  expect(preferenceDefaults.notifications_sound).toBe(true);
  expect(preferenceDefaults.sound).toBe(true);
  expect(preferenceDefaults.session_start_alert).toBe(true);
  expect(preferenceDefaults.session_start_sound).toBe(true);
  expect(preferenceDefaults.warning_minutes).toBe('2');
  render(<Settings />);
  await user.click(screen.getByRole('button', { name: 'Alerts' }));
  expect(screen.queryByRole('switch', { name: 'Play a sound' })).toBeNull();
  const notifSound = screen.getByRole('button', {
    name: 'Notifications sound',
  });
  const finishSound = screen.getByRole('button', {
    name: 'Session finished sound',
  });
  const startSound = screen.getByRole('button', {
    name: 'Session started sound',
  });
  expect(notifSound).toHaveAttribute('aria-pressed', 'true');
  expect(finishSound).toHaveAttribute('aria-pressed', 'true');
  expect(startSound).toHaveAttribute('aria-pressed', 'true');

  // Clicking a bell icon toggles its sound state and saves it.
  await user.click(notifSound);
  expect(notifSound).toHaveAttribute('aria-pressed', 'false');
  await user.click(screen.getByRole('button', { name: 'Save changes' }));
  expect(api.execute).toHaveBeenCalledWith(
    expect.objectContaining({
      type: 'update_preferences',
      preferences: expect.objectContaining({
        notifications_sound: false,
      }),
    }),
  );
});
it('shows block warnings as one minutes box with a short hint', async () => {
  const user = userEvent.setup();
  render(<Settings />);
  await user.click(screen.getByRole('button', { name: 'Alerts' }));
  expect(
    screen.getByText('Warn before a scheduled or one-shot block starts.'),
  ).toBeInTheDocument();
  expect(
    screen.queryByText(/Minutes, separated by commas/),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByText(/Leave blank for no warnings/),
  ).not.toBeInTheDocument();
  const input = screen.getByRole('spinbutton', {
    name: 'Block warning minutes',
  });
  expect(input).toHaveValue(2);
  expect(input.closest('label')).toHaveTextContent('min');
});
it('saves a single block warning minute', async () => {
  const user = userEvent.setup();
  render(<Settings />);
  await user.click(screen.getByRole('button', { name: 'Alerts' }));
  const input = screen.getByRole('spinbutton', {
    name: 'Block warning minutes',
  });
  await user.clear(input);
  await user.type(input, '1');
  await user.click(screen.getByRole('button', { name: 'Save changes' }));
  expect(api.execute).toHaveBeenCalledWith(
    expect.objectContaining({
      type: 'update_preferences',
      preferences: expect.objectContaining({ warning_minutes: '1' }),
    }),
  );
});
it('treats an empty or zero block warning as no warning', async () => {
  const user = userEvent.setup();
  render(<Settings />);
  await user.click(screen.getByRole('button', { name: 'Alerts' }));
  const input = screen.getByRole('spinbutton', {
    name: 'Block warning minutes',
  });
  await user.clear(input);
  await user.click(screen.getByRole('button', { name: 'Save changes' }));
  expect(api.execute).toHaveBeenCalledWith(
    expect.objectContaining({
      type: 'update_preferences',
      preferences: expect.objectContaining({ warning_minutes: '' }),
    }),
  );
  // 0 means the same: no warning, so the box clears.
  await user.type(input, '0');
  expect((input as HTMLInputElement).value).toBe('');
  await user.click(screen.getByRole('button', { name: 'Save changes' }));
  expect(api.execute).toHaveBeenCalledWith(
    expect.objectContaining({
      type: 'update_preferences',
      preferences: expect.objectContaining({ warning_minutes: '' }),
    }),
  );
});
it('shows a stored warning list as one minute and saves one number', async () => {
  const user = userEvent.setup();
  useAppStore.setState({
    data: {
      ...structuredClone(emptySnapshot),
      preferences: {
        theme: 'dark',
        default_minutes: 25,
        warning_minutes: '5, 15',
      },
    },
    loading: false,
    busy: false,
    error: null,
  });
  render(<Settings />);
  await user.click(screen.getByRole('button', { name: 'Alerts' }));
  const input = screen.getByRole('spinbutton', {
    name: 'Block warning minutes',
  });
  expect(input).toHaveValue(5);
  await user.clear(input);
  await user.type(input, '5');
  await user.click(screen.getByRole('button', { name: 'Save changes' }));
  expect(api.execute).toHaveBeenCalledWith(
    expect.objectContaining({
      type: 'update_preferences',
      preferences: expect.objectContaining({ warning_minutes: '5' }),
    }),
  );
});
it('keeps a saved off for the timer and sounds instead of the new defaults', async () => {
  const user = userEvent.setup();
  // A saved off stays off: defaults never override stored values.
  useAppStore.setState({
    data: {
      ...structuredClone(emptySnapshot),
      preferences: {
        theme: 'dark',
        default_minutes: 25,
        menu_timer: false,
        notifications_sound: false,
        sound: false,
        session_start_sound: false,
      },
    },
    loading: false,
    busy: false,
    error: null,
  });
  render(<Settings />);
  expect(
    screen.getByRole('switch', { name: 'Show timer in menu bar' }),
  ).not.toBeChecked();
  await user.click(screen.getByRole('button', { name: 'Alerts' }));
  expect(
    screen.getByRole('button', { name: 'Notifications sound' }),
  ).toHaveAttribute('aria-pressed', 'false');
  expect(
    screen.getByRole('button', { name: 'Session finished sound' }),
  ).toHaveAttribute('aria-pressed', 'false');
  expect(
    screen.getByRole('button', { name: 'Session started sound' }),
  ).toHaveAttribute('aria-pressed', 'false');
});
it('shows connected browser status and refreshes browser connection', async () => {
  const user = userEvent.setup();
  render(<Settings />);
  await user.click(screen.getByRole('button', { name: 'Connections' }));
  expect(await screen.findAllByText('Connected')).toHaveLength(1);
  expect(screen.getAllByText('Allowed')).toHaveLength(1);
  await user.click(screen.getByRole('button', { name: 'Connected' }));
  const refreshBtn = screen.getByRole('button', { name: 'Refresh connection' });
  await user.click(refreshBtn);
  expect(settingsAction).toHaveBeenCalledWith({ type: 'reconnect_browser' });
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

it('offers startup on first run and does not request camera again when granted', async () => {
  const user = userEvent.setup();
  render(<PermissionsGuide onClose={() => {}} />);
  const startup = await screen.findByRole('switch', { name: 'Open at login' });
  await waitFor(() => expect(startup).toBeEnabled());
  await user.click(startup);
  expect(settingsAction).toHaveBeenCalledWith({ type: 'login', enabled: true });
  expect(screen.getByRole('button', { name: 'Allowed' })).toBeDisabled();
  expect(api.requestCamera).not.toHaveBeenCalled();
});

it('flips Open at login in the guide and reflects the saved preference', async () => {
  const user = userEvent.setup();
  let pref = false;
  vi.mocked(settingsAction).mockImplementation(async (action) => {
    if (action.type === 'login') {
      pref = action.enabled;
    }
    return {
      login: pref ? 1 : 0,
      login_user_pref: pref,
      notifications: 0,
    };
  });
  render(<PermissionsGuide onClose={() => {}} />);
  const toggle = await screen.findByRole('switch', { name: 'Open at login' });
  expect(screen.getByText('Opens the app at login.')).toBeInTheDocument();
  expect(
    screen.queryByText(/when there was an active session/),
  ).not.toBeInTheDocument();
  await waitFor(() => expect(toggle).toBeEnabled());
  expect(toggle).not.toBeChecked();
  await user.click(toggle);
  expect(settingsAction).toHaveBeenCalledWith({ type: 'login', enabled: true });
  await waitFor(() =>
    expect(screen.getByRole('switch', { name: 'Open at login' })).toBeChecked(),
  );
  await user.click(screen.getByRole('switch', { name: 'Open at login' }));
  expect(settingsAction).toHaveBeenCalledWith({
    type: 'login',
    enabled: false,
  });
  await waitFor(() =>
    expect(
      screen.getByRole('switch', { name: 'Open at login' }),
    ).not.toBeChecked(),
  );
});

it('toggles Open at login from Settings and saves the preference', async () => {
  const user = userEvent.setup();
  vi.mocked(settingsAction).mockResolvedValue({
    login: 0,
    login_user_pref: false,
    notifications: 0,
  });
  render(<Settings />);
  const toggle = await screen.findByRole('switch', { name: 'Open at login' });
  await waitFor(() => expect(toggle).toBeEnabled());
  expect(toggle).not.toBeChecked();
  await user.click(toggle);
  expect(settingsAction).toHaveBeenCalledWith({ type: 'login', enabled: true });
});

it('renders platform-aware wording on Windows', async () => {
  vi.mocked(settingsAction).mockImplementation(async (action) => {
    if (action.type === 'info') {
      return { platform: 'windows', version: '0.5.4', login: 0 };
    }
    return {};
  });
  render(<Settings />);
  expect(
    await screen.findByRole('switch', { name: 'Show timer in system tray' }),
  ).toBeInTheDocument();
  const user = userEvent.setup();
  await user.click(screen.getByRole('button', { name: 'Data & help' }));
  expect(
    screen.getByText(/the session keeps running in the system tray/),
  ).toBeInTheDocument();
  expect(
    screen.getByText(
      /quit Preelock from the system tray when no session is active/,
    ),
  ).toBeInTheDocument();
});
