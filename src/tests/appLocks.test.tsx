import { beforeEach, expect, it, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { App } from '../App';
import { api, emptySnapshot } from '../services/api';
import { useAppStore } from '../state/store';
import { localDayStart } from '../services/screenTimeWeek';
import type { AppLock, ScreenTimeBucket } from '../types';

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

function seed(buckets: ScreenTimeBucket[], locks: AppLock[] = []) {
  window.location.hash = '#/screen-time';
  useAppStore.setState({
    data: {
      ...structuredClone(emptySnapshot),
      preferences: { ...emptySnapshot.preferences },
      screen_time_usage: buckets,
      app_locks: locks,
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
  vi.mocked(api.snapshot).mockImplementation(async () =>
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
