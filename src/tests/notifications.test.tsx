import { beforeEach, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import { Settings } from '../pages/Settings';
import { Dashboard } from '../pages/Dashboard';
import { useAppStore } from '../state/store';
import { api, emptySnapshot } from '../services/api';
import { settingsAction, preferenceDefaults } from '../services/settings';

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

// The dashboard banner is what step 14 changes; the session cards
// below it are covered by their own tests.
vi.mock('../components/SessionCard', () => ({
  SessionCard: () => null,
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
    browser: {
      available: true,
      connected: true,
      applied_id: '',
      error: '',
      blocks: 0,
      cleanup_pending: false,
      extension_stale: false,
    },
    connected_browsers: [],
    pairing_token: 'test-token',
  });
});

it('shows block warnings and grace period in Alerts and saves them', async () => {
  const user = userEvent.setup();
  render(<Settings />);
  await user.click(screen.getByRole('button', { name: 'Alerts' }));
  const warnings = screen.getByLabelText('Block warning minutes');
  expect(warnings).toHaveValue('5');
  await user.clear(warnings);
  await user.type(warnings, '5, 15');
  const grace = screen.getByLabelText('Grace period seconds');
  expect(grace).toHaveValue(60);
  await user.clear(grace);
  await user.type(grace, '120');
  await user.click(screen.getByRole('button', { name: 'Save changes' }));
  expect(api.execute).toHaveBeenCalledWith(
    expect.objectContaining({
      type: 'update_preferences',
      preferences: expect.objectContaining({
        warning_minutes: '5, 15',
        grace_seconds: 120,
      }),
    }),
  );
});

it('disables block warnings with the master notifications toggle', async () => {
  const user = userEvent.setup();
  render(<Settings />);
  await user.click(screen.getByRole('button', { name: 'Alerts' }));
  await user.click(screen.getByRole('switch', { name: 'Notifications' }));
  expect(screen.getByLabelText('Block warning minutes')).toBeDisabled();
  // Grace is enforcement timing, not an alert: it stays editable
  // while notifications are off.
  expect(screen.getByLabelText('Grace period seconds')).toBeEnabled();
});

it('shows a starting countdown instead of the timer during grace', async () => {
  useAppStore.setState({
    data: {
      ...structuredClone(emptySnapshot),
      preferences: { ...preferenceDefaults },
      sessions: [
        {
          id: 'live',
          title: 'Morning block',
          duration_minutes: 60,
          remaining_ms: 60 * 60_000,
          focused_ms: 0,
          allow_pause: false,
          status: 'running',
          started_at: Date.now(),
          ended_at: null,
          deadline_ms: null,
          grace_until_ms: Date.now() + 45_000,
        },
      ],
      browser: {
        available: true,
        connected: true,
        applied_id: '',
        error: '',
        blocks: 0,
        cleanup_pending: false,
        extension_stale: false,
      },
    },
  });
  render(
    <MemoryRouter>
      <Dashboard />
    </MemoryRouter>,
  );
  expect(await screen.findByText(/starting soon/i)).toBeInTheDocument();
  expect(screen.getByText(/Starts in \d+s/)).toBeInTheDocument();
  expect(
    screen.getByText(/nothing is blocked yet/i),
  ).toBeInTheDocument();
});
