import { beforeEach, expect, it, vi } from 'vitest';
import { act, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { App } from '../App';
import { api, emptySnapshot } from '../services/api';
import { useAppStore } from '../state/store';
import { localDayStart } from '../services/screenTimeWeek';
import type {
  AppLock,
  AdultBlock,
  ScreenTimeBucket,
  StrictBlockState,
} from '../types';

vi.mock('../services/settings', () => ({
  settingsAction: vi.fn().mockResolvedValue({}),
  preferenceDefaults: {
    theme: 'dark',
    default_minutes: 25,
    saved_setups: [],
    reminder_days: [],
  },
}));

vi.mock('../services/api', () => ({
  desktop: true,
  emptySnapshot: {
    sessions: [],
    tasks: [],
    schedules: [],
    preferences: { theme: 'dark', default_minutes: 25 },
  },
  api: {
    snapshot: vi.fn(),
    execute: vi.fn(),
    focusSetup: vi.fn(),
    capabilities: vi.fn(),
    blockAttempts: vi.fn().mockResolvedValue([]),
    appIcon: vi.fn().mockResolvedValue(null),
  },
}));

function bucket(
  name: string,
  target: string,
  minutes: number,
  kind: 'app' | 'site' = 'app',
): ScreenTimeBucket {
  return {
    day_ms: localDayStart(new Date()),
    kind,
    target,
    name,
    ms: minutes * 60_000,
  };
}

function seed(
  buckets: ScreenTimeBucket[],
  locks: AppLock[] = [],
  adultBlock?: AdultBlock,
  strictApps?: StrictBlockState,
  strictSites?: StrictBlockState,
) {
  window.location.hash = '#/screen-time';
  useAppStore.setState({
    data: {
      ...structuredClone(emptySnapshot),
      preferences: { ...emptySnapshot.preferences },
      screen_time_usage: buckets,
      app_locks: locks,
      adult_block: adultBlock ?? {
        enabled: false,
        strict: false,
        ends_at_ms: null,
        activating_until_ms: null,
        created_at_ms: 0,
      },
      strict_apps: strictApps ?? {
        enabled: false,
        strict: false,
        ends_at_ms: null,
        activating_until_ms: null,
        created_at_ms: 0,
      },
      strict_sites: strictSites ?? {
        enabled: false,
        strict: false,
        ends_at_ms: null,
        activating_until_ms: null,
        created_at_ms: 0,
      },
    },
    loading: false,
    error: null,
    busy: false,
  });
  render(<App />);
}

// The backend round-trips the created lock back in the snapshot.
function mockCreateAppLock() {
  vi.mocked(api.execute).mockImplementation(async (action) => {
    const current = useAppStore.getState().data;
    if (action.type === 'create_app_lock') {
      const lock: AppLock = {
        id: 'new-lock',
        title: action.title,
        kind: action.kind,
        target: action.target,
        rule:
          action.rule.kind === 'timed'
            ? {
                type: 'timed',
                ends_at_ms: Date.now() + action.rule.minutes * 60_000,
              }
            : {
                type: 'daily_limit',
                limit_ms: action.rule.limit_minutes * 60_000,
              },
        enabled: true,
        created_at: Date.now(),
      };
      return { ...current, app_locks: [...(current.app_locks ?? []), lock] };
    }
    return current;
  });
}

function lockSection(): HTMLElement {
  return screen.getByText('App lock').closest('section') as HTMLElement;
}

beforeEach(() => {
  window.location.hash = '#/screen-time';
  localStorage.setItem('preelock-permissions-v2', 'seen');
  vi.clearAllMocks();
  HTMLDialogElement.prototype.showModal = function () {
    this.setAttribute('open', '');
  };
  HTMLDialogElement.prototype.close = function () {
    this.removeAttribute('open');
  };
  vi.mocked(api.snapshot).mockImplementation(async () =>
    structuredClone(useAppStore.getState().data),
  );
  vi.mocked(api.execute).mockImplementation(async () =>
    structuredClone(useAppStore.getState().data),
  );
  vi.mocked(api.capabilities).mockResolvedValue([]);
  vi.mocked(api.focusSetup).mockResolvedValue({
    platform: 'macos',
    native_available: true,
    applications: [],
    camera_permission: 'not_requested',
    pairing_token: 'test-token',
    browser: {
      available: true,
      connected: false,
      applied_id: '',
      error: '',
      blocks: 0,
      cleanup_pending: false,
      extension_stale: false,
    },
  });
});

it('expands and collapses the Block apps panel', async () => {
  seed([bucket('Alpha App', 'com.example.alpha', 60)]);
  const user = userEvent.setup();
  await screen.findByText('App lock');
  const toggle = screen.getByRole('button', { name: 'Block apps' });
  expect(toggle).toHaveAttribute('aria-expanded', 'false');
  expect(
    screen.queryByRole('button', { name: 'Block for a set time' }),
  ).not.toBeInTheDocument();
  await user.click(toggle);
  expect(toggle).toHaveAttribute('aria-expanded', 'true');
  expect(
    screen.getByRole('button', { name: 'Block for a set time' }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole('button', { name: 'Block after daily limit' }),
  ).toBeInTheDocument();
  expect(
    within(lockSection()).getByText('No active app blocks.'),
  ).toBeInTheDocument();
  await user.click(toggle);
  expect(toggle).toHaveAttribute('aria-expanded', 'false');
  expect(
    screen.queryByRole('button', { name: 'Block for a set time' }),
  ).not.toBeInTheDocument();
});

it('opening one panel closes the other', async () => {
  seed([bucket('Alpha App', 'com.example.alpha', 60)]);
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(screen.getByRole('button', { name: 'Block apps' }));
  expect(
    within(lockSection()).getByText('No active app blocks.'),
  ).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Block websites' }));
  expect(
    within(lockSection()).queryByText('No active app blocks.'),
  ).not.toBeInTheDocument();
  expect(
    within(lockSection()).getByText('No active website blocks.'),
  ).toBeInTheDocument();
});

it('the app picker lists recently used apps, most used first', async () => {
  seed([
    bucket('Beta App', 'com.example.beta', 30),
    bucket('Alpha App', 'com.example.alpha', 60),
  ]);
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(screen.getByRole('button', { name: 'Block apps' }));
  await user.click(
    screen.getByRole('button', { name: 'Block for a set time' }),
  );
  // The trigger defaults to the most-used app.
  const trigger = screen.getByRole('button', { name: 'Alpha App' });
  await user.click(trigger);
  expect(
    screen.getAllByRole('option').map((option) => option.textContent),
  ).toEqual(['Alpha App', 'Beta App']);
  await user.click(screen.getByRole('option', { name: 'Beta App' }));
  expect(screen.getByRole('button', { name: 'Beta App' })).toBeInTheDocument();
});

it('creates a timed app block with a preset duration', async () => {
  seed([
    bucket('Alpha App', 'com.example.alpha', 60),
    bucket('Beta App', 'com.example.beta', 30),
  ]);
  mockCreateAppLock();
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(screen.getByRole('button', { name: 'Block apps' }));
  await user.click(
    screen.getByRole('button', { name: 'Block for a set time' }),
  );
  await user.click(screen.getByRole('button', { name: '5 hours' }));
  await user.click(screen.getByRole('button', { name: 'Start block' }));
  expect(api.execute).toHaveBeenCalledWith({
    type: 'create_app_lock',
    title: 'Alpha App',
    kind: 'app',
    target: 'com.example.alpha',
    rule: { kind: 'timed', minutes: 300 },
  });
  // The form collapses and the new lock appears in the Active list.
  const section = lockSection();
  await within(section).findByText('5h left');
  expect(
    within(section).queryByRole('button', { name: 'Start block' }),
  ).not.toBeInTheDocument();
  expect(within(section).getByText('Alpha App')).toBeInTheDocument();
  expect(within(section).getByText('Timed')).toBeInTheDocument();
});

it('creates a timed block with custom minutes', async () => {
  seed([bucket('Alpha App', 'com.example.alpha', 60)]);
  mockCreateAppLock();
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(screen.getByRole('button', { name: 'Block apps' }));
  await user.click(
    screen.getByRole('button', { name: 'Block for a set time' }),
  );
  const custom = screen.getByLabelText('Custom (minutes)');
  await user.clear(custom);
  await user.type(custom, '90');
  await user.click(screen.getByRole('button', { name: 'Start block' }));
  expect(api.execute).toHaveBeenCalledWith({
    type: 'create_app_lock',
    title: 'Alpha App',
    kind: 'app',
    target: 'com.example.alpha',
    rule: { kind: 'timed', minutes: 90 },
  });
});

it('strips a leading zero from custom minutes (no stuck "010")', async () => {
  seed([bucket('Alpha App', 'com.example.alpha', 60)]);
  mockCreateAppLock();
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(screen.getByRole('button', { name: 'Block apps' }));
  await user.click(
    screen.getByRole('button', { name: 'Block for a set time' }),
  );
  const custom = screen.getByLabelText('Custom (minutes)') as HTMLInputElement;
  await user.clear(custom);
  await user.type(custom, '010');
  // The zero is stripped as typed: the field shows "10", not "010".
  expect(custom.value).toBe('10');
  await user.click(screen.getByRole('button', { name: 'Start block' }));
  expect(api.execute).toHaveBeenCalledWith({
    type: 'create_app_lock',
    title: 'Alpha App',
    kind: 'app',
    target: 'com.example.alpha',
    rule: { kind: 'timed', minutes: 10 },
  });
});

it('allows clearing the custom-minutes field and keeps a single zero', async () => {
  seed([bucket('Alpha App', 'com.example.alpha', 60)]);
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(screen.getByRole('button', { name: 'Block apps' }));
  await user.click(
    screen.getByRole('button', { name: 'Block for a set time' }),
  );
  const custom = screen.getByLabelText('Custom (minutes)') as HTMLInputElement;
  await user.clear(custom);
  expect(custom.value).toBe('');
  await user.type(custom, '0');
  expect(custom.value).toBe('0');
});

it('creates a daily-limit website block with a normalized domain', async () => {
  seed([]);
  mockCreateAppLock();
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(screen.getByRole('button', { name: 'Block websites' }));
  await user.click(
    screen.getByRole('button', { name: 'Block after daily limit' }),
  );
  await user.type(
    screen.getByPlaceholderText('e.g. youtube.com'),
    'https://YouTube.com/watch?v=1',
  );
  await user.click(screen.getByRole('button', { name: '2 hours' }));
  await user.click(screen.getByRole('button', { name: 'Set limit' }));
  expect(api.execute).toHaveBeenCalledWith({
    type: 'create_app_lock',
    title: 'youtube.com',
    kind: 'site',
    target: 'youtube.com',
    rule: { kind: 'daily_limit', limit_minutes: 120 },
  });
});

it('shows the backend error under the form when creation fails', async () => {
  seed([bucket('Alpha App', 'com.example.alpha', 60)]);
  vi.mocked(api.execute).mockRejectedValue(
    new Error('Preelock itself and system apps cannot be app-locked.'),
  );
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(screen.getByRole('button', { name: 'Block apps' }));
  await user.click(
    screen.getByRole('button', { name: 'Block for a set time' }),
  );
  await user.click(screen.getByRole('button', { name: 'Start block' }));
  const form = screen
    .getByRole('button', { name: 'Start block' })
    .closest('form') as HTMLElement;
  expect(within(form).getByRole('alert')).toHaveTextContent(
    'Preelock itself and system apps cannot be app-locked.',
  );
  expect(api.execute).toHaveBeenCalledTimes(1);
});

it('renders active locks with live status text, newest first', async () => {
  const now = Date.now();
  const locks: AppLock[] = [
    {
      id: 'lock-daily',
      title: 'youtube.com',
      kind: 'site',
      target: 'youtube.com',
      rule: { type: 'daily_limit', limit_ms: 120 * 60_000 },
      enabled: true,
      created_at: now - 120_000,
    },
    {
      id: 'lock-timed',
      title: 'Game',
      kind: 'app',
      target: 'com.example.game',
      rule: { type: 'timed', ends_at_ms: now + 133 * 60_000 + 30_000 },
      enabled: true,
      created_at: now - 60_000,
    },
  ];
  seed(
    [
      bucket('Game', 'com.example.game', 10),
      bucket('youtube.com', 'youtube.com', 45, 'site'),
    ],
    locks,
  );
  const user = userEvent.setup();
  await screen.findByText('App lock');
  // The apps panel lists only app locks, with the timed countdown.
  await user.click(screen.getByRole('button', { name: 'Block apps' }));
  const section = lockSection();
  expect(within(section).getByText('Game')).toBeInTheDocument();
  expect(within(section).getByText('Timed')).toBeInTheDocument();
  expect(within(section).getByText('2h 13m left')).toBeInTheDocument();
  expect(within(section).queryByText('youtube.com')).not.toBeInTheDocument();
  // The websites panel lists only site locks, with today's usage.
  await user.click(screen.getByRole('button', { name: 'Block websites' }));
  expect(within(section).getByText('youtube.com')).toBeInTheDocument();
  expect(within(section).getByText('Daily limit')).toBeInTheDocument();
  expect(within(section).getByText('45m of 2h used today')).toBeInTheDocument();
  expect(within(section).queryByText('Game')).not.toBeInTheDocument();
});

it('shows blocking status when a daily limit is already exceeded', async () => {
  const now = Date.now();
  seed(
    [bucket('youtube.com', 'youtube.com', 150, 'site')],
    [
      {
        id: 'lock-daily',
        title: 'youtube.com',
        kind: 'site',
        target: 'youtube.com',
        rule: { type: 'daily_limit', limit_ms: 120 * 60_000 },
        enabled: true,
        created_at: now - 60_000,
      },
    ],
  );
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(screen.getByRole('button', { name: 'Block websites' }));
  expect(
    within(lockSection()).getByText('Limit reached — blocking until midnight'),
  ).toBeInTheDocument();
});

it('remove dispatches delete_app_lock', async () => {
  const now = Date.now();
  seed(
    [],
    [
      {
        id: 'lock-1',
        title: 'Game',
        kind: 'app',
        target: 'com.example.game',
        rule: { type: 'timed', ends_at_ms: now + 60 * 60_000 },
        enabled: true,
        created_at: now,
      },
    ],
  );
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(screen.getByRole('button', { name: 'Block apps' }));
  await user.click(screen.getByRole('button', { name: 'Remove block: Game' }));
  expect(api.execute).toHaveBeenCalledWith({
    type: 'delete_app_lock',
    id: 'lock-1',
  });
});

it('renders Block adult websites option with turn on toggle and dispatches set_adult_block', async () => {
  seed([]);
  const user = userEvent.setup();
  await screen.findByText('App lock');
  const adultButton = screen.getByRole('button', {
    name: 'Block adult websites',
  });
  expect(adultButton).toBeInTheDocument();
  await user.click(adultButton);

  const toggle = screen.getByRole('switch', { name: 'Block adult websites' });
  expect(toggle).toBeInTheDocument();
  expect(toggle).toHaveAttribute('aria-checked', 'false');

  await user.click(toggle);
  expect(api.execute).toHaveBeenCalledWith({
    type: 'set_adult_block',
    enabled: true,
    strict: false,
  });
});

it('when enabled, renders compact strict row with day presets, and allows 20s countdown cancellation', async () => {
  const now = Date.now();
  seed([], [], {
    enabled: true,
    strict: false,
    ends_at_ms: null,
    activating_until_ms: null,
    created_at_ms: now,
  });
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(
    screen.getByRole('button', { name: 'Block adult websites' }),
  );

  const mainToggle = screen.getByRole('switch', {
    name: 'Block adult websites',
  });
  expect(mainToggle).toHaveAttribute('aria-checked', 'true');

  const strictToggle = screen.getByRole('switch', { name: 'Strict block' });
  expect(strictToggle).toBeInTheDocument();
  expect(strictToggle).toHaveAttribute('aria-checked', 'false');

  // Day preset chips are present beside the toggle
  expect(screen.getByRole('button', { name: '1 day' })).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '3 days' })).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '7 days' })).toBeInTheDocument();

  // Select 3 days
  await user.click(screen.getByRole('button', { name: '3 days' }));
  expect(screen.getByRole('button', { name: '3 days' })).toHaveAttribute(
    'aria-pressed',
    'true',
  );

  // Turn on strict toggle -> dispatches start_strict_block with days = 3
  await user.click(strictToggle);
  expect(api.execute).toHaveBeenCalledWith({
    type: 'start_strict_block',
    target: 'adult',
    days: 3,
  });
});

it('renders persistent 20s countdown card and cancels via cancel_strict_block', async () => {
  const now = Date.now();
  seed([], [], {
    enabled: true,
    strict: false,
    ends_at_ms: now + 20_000 + 3 * 86_400_000,
    activating_until_ms: now + 20_000,
    created_at_ms: now,
  });
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(
    screen.getByRole('button', { name: 'Block adult websites' }),
  );

  expect(screen.getByText(/Strict block activating in/)).toBeInTheDocument();
  expect(
    screen.getByText(/Filters adult websites strictly for 3 days\./),
  ).toBeInTheDocument();

  const cancelBtn = screen.getByRole('button', { name: 'Turn back off' });
  await user.click(cancelBtn);
  expect(api.execute).toHaveBeenCalledWith({
    type: 'cancel_strict_block',
    target: 'adult',
  });
});

it('strict block displays active state with locked notice and disables turning off', async () => {
  const now = Date.now();
  seed([], [], {
    enabled: true,
    strict: true,
    ends_at_ms: now + 86400000,
    activating_until_ms: null,
    created_at_ms: now,
  });
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(
    screen.getByRole('button', { name: 'Block adult websites' }),
  );
  expect(
    screen.getByText('Adult websites blocked system-wide'),
  ).toBeInTheDocument();
  expect(screen.getByText('Strict block')).toBeInTheDocument();
  expect(
    screen.getByText(/It cannot be canceled or disabled until expired\./),
  ).toBeInTheDocument();

  const toggle = screen.getByRole('switch', { name: 'Block adult websites' });
  expect(toggle).toHaveAttribute('aria-checked', 'true');
  expect(toggle).toBeDisabled();
});

it('strict block in Block apps asks first, then locks the app lock and removes delete button', async () => {
  const now = Date.now();
  const sampleLock: AppLock = {
    id: 'lock-app-1',
    title: 'Safari',
    kind: 'app',
    target: 'com.apple.Safari',
    rule: { type: 'timed', ends_at_ms: now + 45 * 60_000 + 30_000 },
    created_at: now,
  };

  seed([], [sampleLock]);
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(screen.getByRole('button', { name: 'Block apps' }));

  // Strict block controls are present in active block row
  const strictToggle = screen.getByRole('switch', {
    name: 'Strict block for Safari',
  });
  expect(strictToggle).toBeInTheDocument();
  expect(strictToggle).toHaveAttribute('aria-checked', 'false');

  // Active lock shows delete button initially
  expect(
    screen.getByRole('button', { name: 'Remove block: Safari' }),
  ).toBeInTheDocument();

  // Clicking the toggle only opens the confirmation — Strict stays off
  // and nothing is sent to the backend yet.
  await user.click(strictToggle);
  expect(api.execute).not.toHaveBeenCalled();
  const dialog = await screen.findByRole('dialog', { name: 'Strict block' });
  expect(dialog).toHaveTextContent(
    'Safari will be strictly blocked for the next 45m. This cannot be undone.',
  );
  expect(strictToggle).toHaveAttribute('aria-checked', 'false');

  // Block confirms: dispatches set_app_lock_strict and closes the popup.
  await user.click(within(dialog).getByRole('button', { name: 'Block' }));
  expect(api.execute).toHaveBeenCalledWith({
    type: 'set_app_lock_strict',
    id: 'lock-app-1',
  });
  expect(screen.queryByRole('dialog', { name: 'Strict block' })).toBeNull();

  // When strict mode is enabled on the lock:
  act(() => {
    useAppStore.setState((s) => ({
      data: {
        ...s.data,
        app_locks: [{ ...sampleLock, strict: true }],
      },
    }));
  });

  // Delete button is removed and toggle is checked & disabled
  expect(
    screen.queryByRole('button', { name: 'Remove block: Safari' }),
  ).toBeNull();
  expect(
    screen.getByRole('switch', { name: 'Strict block for Safari' }),
  ).toHaveAttribute('aria-checked', 'true');
  expect(
    screen.getByRole('switch', { name: 'Strict block for Safari' }),
  ).toBeDisabled();
});

it('strict block in Block websites asks first, then locks the website lock and removes delete button', async () => {
  const now = Date.now();
  const sampleLock: AppLock = {
    id: 'lock-site-1',
    title: 'youtube.com',
    kind: 'site',
    target: 'youtube.com',
    rule: { type: 'timed', ends_at_ms: now + 45 * 60_000 + 30_000 },
    created_at: now,
  };

  seed([], [sampleLock]);
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(screen.getByRole('button', { name: 'Block websites' }));

  // Strict block controls are present in active block row
  const strictToggle = screen.getByRole('switch', {
    name: 'Strict block for youtube.com',
  });
  expect(strictToggle).toBeInTheDocument();
  expect(strictToggle).toHaveAttribute('aria-checked', 'false');

  // Active lock shows delete button initially
  expect(
    screen.getByRole('button', { name: 'Remove block: youtube.com' }),
  ).toBeInTheDocument();

  // Clicking the toggle only opens the confirmation — Strict stays off
  // and nothing is sent to the backend yet.
  await user.click(strictToggle);
  expect(api.execute).not.toHaveBeenCalled();
  const dialog = await screen.findByRole('dialog', { name: 'Strict block' });
  expect(dialog).toHaveTextContent(
    'youtube.com will be strictly blocked for the next 45m. This cannot be undone.',
  );
  expect(strictToggle).toHaveAttribute('aria-checked', 'false');

  // Block confirms: dispatches set_app_lock_strict and closes the popup.
  await user.click(within(dialog).getByRole('button', { name: 'Block' }));
  expect(api.execute).toHaveBeenCalledWith({
    type: 'set_app_lock_strict',
    id: 'lock-site-1',
  });
  expect(screen.queryByRole('dialog', { name: 'Strict block' })).toBeNull();

  // When strict mode is enabled on the lock:
  act(() => {
    useAppStore.setState((s) => ({
      data: {
        ...s.data,
        app_locks: [{ ...sampleLock, strict: true }],
      },
    }));
  });

  // Delete button is removed and toggle is checked & disabled
  expect(
    screen.queryByRole('button', { name: 'Remove block: youtube.com' }),
  ).toBeNull();
  expect(
    screen.getByRole('switch', { name: 'Strict block for youtube.com' }),
  ).toHaveAttribute('aria-checked', 'true');
  expect(
    screen.getByRole('switch', { name: 'Strict block for youtube.com' }),
  ).toBeDisabled();
});

it('cancel in the strict confirmation leaves the block untouched', async () => {
  const now = Date.now();
  const sampleLock: AppLock = {
    id: 'lock-app-1',
    title: 'Safari',
    kind: 'app',
    target: 'com.apple.Safari',
    rule: { type: 'timed', ends_at_ms: now + 45 * 60_000 + 30_000 },
    created_at: now,
  };

  seed([], [sampleLock]);
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(screen.getByRole('button', { name: 'Block apps' }));

  const strictToggle = screen.getByRole('switch', {
    name: 'Strict block for Safari',
  });
  await user.click(strictToggle);
  const dialog = await screen.findByRole('dialog', { name: 'Strict block' });

  // Cancel closes the popup and sends nothing to the backend.
  await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
  expect(api.execute).not.toHaveBeenCalled();
  expect(screen.queryByRole('dialog', { name: 'Strict block' })).toBeNull();
  expect(strictToggle).toHaveAttribute('aria-checked', 'false');
  expect(
    screen.getByRole('button', { name: 'Remove block: Safari' }),
  ).toBeInTheDocument();
});

it('daily-limit strict confirmation names the rest of today, not a duration', async () => {
  const now = Date.now();
  seed(
    [bucket('youtube.com', 'youtube.com', 45, 'site')],
    [
      {
        id: 'lock-daily',
        title: 'youtube.com',
        kind: 'site',
        target: 'youtube.com',
        rule: { type: 'daily_limit', limit_ms: 120 * 60_000 },
        enabled: true,
        created_at: now - 60_000,
      },
    ],
  );
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(screen.getByRole('button', { name: 'Block websites' }));

  await user.click(
    screen.getByRole('switch', { name: 'Strict block for youtube.com' }),
  );
  const dialog = await screen.findByRole('dialog', { name: 'Strict block' });
  expect(dialog).toHaveTextContent(
    'youtube.com will be strictly blocked for the rest of today once the daily limit is reached. This cannot be undone.',
  );
  expect(api.execute).not.toHaveBeenCalled();
});

it('daily-limit strict confirmation says until midnight once the limit is reached', async () => {
  const now = Date.now();
  seed(
    [bucket('youtube.com', 'youtube.com', 150, 'site')],
    [
      {
        id: 'lock-daily',
        title: 'youtube.com',
        kind: 'site',
        target: 'youtube.com',
        rule: { type: 'daily_limit', limit_ms: 120 * 60_000 },
        enabled: true,
        created_at: now - 60_000,
      },
    ],
  );
  const user = userEvent.setup();
  await screen.findByText('App lock');
  await user.click(screen.getByRole('button', { name: 'Block websites' }));

  await user.click(
    screen.getByRole('switch', { name: 'Strict block for youtube.com' }),
  );
  const dialog = await screen.findByRole('dialog', { name: 'Strict block' });
  expect(dialog).toHaveTextContent(
    'youtube.com will be strictly blocked until midnight. This cannot be undone.',
  );
  expect(api.execute).not.toHaveBeenCalled();
});
