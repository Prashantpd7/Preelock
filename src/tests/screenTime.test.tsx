import { beforeEach, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { App } from '../App';
import { api, emptySnapshot } from '../services/api';
import { useAppStore } from '../state/store';
import {
  formatDayLabel,
  localDayStart,
} from '../services/screenTimeWeek';
import type { ScreenTimeBucket } from '../types';

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
  minutes: number,
  dayOffset = 0,
): ScreenTimeBucket {
  const day = new Date();
  day.setDate(day.getDate() + dayOffset);
  day.setHours(0, 0, 0, 0);
  return {
    day_ms: day.getTime(),
    kind: 'app',
    target: `com.example.${name.split(' ')[0].toLowerCase()}`,
    name,
    ms: minutes * 60_000,
  };
}

function seed(
  names: [string, number, number?][],
  prefs?: Record<string, unknown>,
) {
  window.location.hash = '#/screen-time';
  useAppStore.setState({
    data: {
      ...structuredClone(emptySnapshot),
      preferences: { ...emptySnapshot.preferences, ...prefs },
      screen_time_usage: names.map(([name, minutes, dayOffset]) =>
        bucket(name, minutes, dayOffset ?? 0),
      ),
    },
    loading: false,
    error: null,
    busy: false,
  });
  render(<App />);
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

it('shows only the top 4 apps until Show all is clicked', async () => {
  seed([
    ['Alpha App', 60],
    ['Beta App', 50],
    ['Gamma App', 40],
    ['Delta App', 30],
    ['Echo App', 20],
    ['Foxtrot App', 10],
  ]);
  const user = userEvent.setup();
  await screen.findByText('Apps and websites');
  expect(screen.getByText('Alpha App')).toBeInTheDocument();
  expect(screen.getByText('Delta App')).toBeInTheDocument();
  expect(screen.queryByText('Echo App')).not.toBeInTheDocument();
  expect(screen.queryByText('Foxtrot App')).not.toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Show all' }));
  expect(screen.getByText('Echo App')).toBeInTheDocument();
  expect(screen.getByText('Foxtrot App')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Show less' }));
  expect(screen.queryByText('Echo App')).not.toBeInTheDocument();
  expect(screen.queryByText('Foxtrot App')).not.toBeInTheDocument();
  expect(
    screen.getByRole('button', { name: 'Show all' }),
  ).toBeInTheDocument();
});

it('shows no button for 4 or fewer rows', async () => {
  seed([
    ['Alpha App', 60],
    ['Beta App', 50],
    ['Gamma App', 40],
  ]);
  await screen.findByText('Apps and websites');
  expect(screen.getByText('Gamma App')).toBeInTheDocument();
  expect(
    screen.queryByRole('button', { name: 'Show all' }),
  ).not.toBeInTheDocument();
});

it('collapses back to the top 4 when the selected day changes', async () => {
  seed([
    ['Alpha App', 60],
    ['Beta App', 50],
    ['Gamma App', 40],
    ['Delta App', 30],
    ['Echo App', 20],
    ['Foxtrot App', 10],
    ['Yesteryear App', 45, -1],
    ['Prior App', 15, -1],
  ]);
  const user = userEvent.setup();
  await screen.findByText('Apps and websites');
  await user.click(screen.getByRole('button', { name: 'Show all' }));
  expect(screen.getByText('Foxtrot App')).toBeInTheDocument();
  const yesterday = new Date();
  yesterday.setDate(yesterday.getDate() - 1);
  await user.click(
    screen.getByRole('button', {
      name: formatDayLabel(localDayStart(yesterday)),
    }),
  );
  expect(screen.queryByText('Echo App')).not.toBeInTheDocument();
  expect(screen.queryByText('Foxtrot App')).not.toBeInTheDocument();
  expect(screen.getByText('Yesteryear App')).toBeInTheDocument();
  expect(screen.getByText('Prior App')).toBeInTheDocument();
  expect(
    screen.queryByRole('button', { name: 'Show all' }),
  ).not.toBeInTheDocument();
});

it('disables the next-week arrow on the current week and re-selects today when returning', async () => {
  seed([
    ['Alpha App', 60],
    ['Beta App', 50],
  ]);
  const user = userEvent.setup();
  await screen.findByText('Apps and websites');
  expect(screen.getByRole('button', { name: 'Next week' })).toBeDisabled();
  await user.click(screen.getByRole('button', { name: 'Previous week' }));
  expect(screen.getByRole('button', { name: 'Next week' })).toBeEnabled();
  expect(screen.queryByText('Alpha App')).not.toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Next week' }));
  expect(screen.getByRole('button', { name: 'Next week' })).toBeDisabled();
  expect(screen.getByText('Alpha App')).toBeInTheDocument();
});

it('saves a category correction from the row menu and applies it at once', async () => {
  seed([['Alpha App', 60]]);
  const user = userEvent.setup();
  await screen.findByText('Apps and websites');
  // Alpha App is unknown, so it starts as Others.
  await user.click(screen.getByRole('button', { name: 'Others' }));
  expect(
    screen.getByRole('option', { name: 'Distracting' }),
  ).toBeInTheDocument();
  // The backend round-trips the saved preferences back in the snapshot.
  vi.mocked(api.execute).mockImplementation(async (action) => {
    const current = useAppStore.getState().data;
    if (action.type === 'update_preferences') {
      return { ...current, preferences: action.preferences };
    }
    return current;
  });
  await user.click(screen.getByRole('option', { name: 'Distracting' }));
  expect(api.execute).toHaveBeenCalledWith({
    type: 'update_preferences',
    preferences: expect.objectContaining({
      category_overrides: { 'com.example.alpha': 'Distracting' },
    }),
  });
  // The row label updates immediately, without a restart.
  expect(
    screen.getByRole('button', { name: 'Distracting' }),
  ).toBeInTheDocument();
});

it('closes the category menu on Escape without saving', async () => {
  seed([['Alpha App', 60]]);
  const user = userEvent.setup();
  await screen.findByText('Apps and websites');
  await user.click(screen.getByRole('button', { name: 'Others' }));
  expect(
    screen.getByRole('option', { name: 'Distracting' }),
  ).toBeInTheDocument();
  await user.keyboard('{Escape}');
  expect(
    screen.queryByRole('option', { name: 'Distracting' }),
  ).not.toBeInTheDocument();
  expect(api.execute).not.toHaveBeenCalled();
});

it('renders the App lock section with both options below the usage list', async () => {
  seed([['Alpha App', 60]]);
  await screen.findByText('Apps and websites');
  expect(screen.getByText('App lock')).toBeInTheDocument();
  // Both options are expanders; expanding one calls no backend action.
  const blockApps = screen.getByRole('button', { name: 'Block apps' });
  const blockWebsites = screen.getByRole('button', { name: 'Block websites' });
  expect(blockApps).toHaveAttribute('aria-expanded', 'false');
  expect(blockWebsites).toHaveAttribute('aria-expanded', 'false');
  const user = userEvent.setup();
  await user.click(blockApps);
  expect(blockApps).toHaveAttribute('aria-expanded', 'true');
  expect(
    screen.getByRole('button', { name: 'Block for a set time' }),
  ).toBeInTheDocument();
  expect(api.execute).not.toHaveBeenCalled();
});
