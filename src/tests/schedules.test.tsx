import { beforeEach, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { App } from '../App';
import { api, emptySnapshot } from '../services/api';
import { useAppStore } from '../state/store';
import { defaultFocus, type Schedule, type Session } from '../types';
import {
  formatDays,
  isScheduleActive,
  validateScheduleForm,
} from '../services/schedules';

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
  },
}));

const templateSession: Session = {
  id: 'sess-1',
  title: 'Write outline',
  duration_minutes: 25,
  remaining_ms: 25 * 60_000,
  focused_ms: 0,
  allow_pause: true,
  status: 'ready',
  started_at: null,
  ended_at: null,
  deadline_ms: null,
  focus: structuredClone(defaultFocus),
};

function schedule(overrides: Partial<Schedule>): Schedule {
  return {
    id: 'sched-1',
    title: 'Morning block',
    days: [1, 2, 3, 4, 5],
    start_time: '09:00',
    end_time: '10:00',
    focus: structuredClone(defaultFocus),
    enabled: true,
    lock_changes: false,
    created_at: 1,
    ...overrides,
  };
}

beforeEach(() => {
  localStorage.setItem('preelock-permissions-v2', 'seen');
  window.location.hash = '#/sessions';
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

it('matches same-day windows with start-inclusive, end-exclusive bounds', () => {
  const s = schedule({ days: [1, 3], start_time: '09:00', end_time: '10:00' });
  // Monday 2026-10-26 is a Monday; use explicit local dates.
  const monday = new Date(2026, 9, 26);
  expect(isScheduleActive(s, new Date(2026, 9, 26, 9, 0))).toBe(true);
  expect(isScheduleActive(s, new Date(2026, 9, 26, 9, 59))).toBe(true);
  expect(isScheduleActive(s, new Date(2026, 9, 26, 10, 0))).toBe(false);
  expect(isScheduleActive(s, new Date(2026, 9, 26, 8, 59))).toBe(false);
  expect(isScheduleActive(s, new Date(2026, 9, 27, 9, 30))).toBe(false);
  expect(isScheduleActive(s, new Date(2026, 9, 28, 9, 30))).toBe(true);
  expect(monday.getDay()).toBe(1);
});

it('matches overnight windows across midnight on the right days', () => {
  const s = schedule({ days: [1], start_time: '22:00', end_time: '06:00' });
  expect(isScheduleActive(s, new Date(2026, 9, 26, 22, 0))).toBe(true);
  expect(isScheduleActive(s, new Date(2026, 9, 26, 23, 59))).toBe(true);
  expect(isScheduleActive(s, new Date(2026, 9, 27, 0, 0))).toBe(true);
  expect(isScheduleActive(s, new Date(2026, 9, 27, 5, 59))).toBe(true);
  expect(isScheduleActive(s, new Date(2026, 9, 27, 6, 0))).toBe(false);
  expect(isScheduleActive(s, new Date(2026, 9, 27, 22, 0))).toBe(false);
  expect(isScheduleActive(s, new Date(2026, 9, 26, 5, 0))).toBe(false);
});

it('validates the schedule form', () => {
  const base = {
    title: 'Block',
    days: [1],
    start_time: '09:00',
    end_time: '10:00',
  };
  expect(validateScheduleForm(base)).toBe('');
  expect(validateScheduleForm({ ...base, days: [] })).toContain('day');
  expect(validateScheduleForm({ ...base, title: '  ' })).toContain('title');
  expect(validateScheduleForm({ ...base, end_time: '09:00' })).toContain(
    'differ',
  );
  expect(validateScheduleForm({ ...base, start_time: '25:00' })).toContain(
    'HH:MM',
  );
  // Overnight is valid, not an error.
  expect(
    validateScheduleForm({ ...base, start_time: '22:00', end_time: '06:00' }),
  ).toBe('');
});

it('formats day lists', () => {
  expect(formatDays([1, 2, 3, 4, 5])).toBe('Weekdays');
  expect(formatDays([0, 6])).toBe('Weekends');
  expect(formatDays([0, 1, 2, 3, 4, 5, 6])).toBe('Every day');
  expect(formatDays([1, 3])).toBe('Mon, Wed');
});

it('lists schedules with a lock indicator and disables locked toggles', async () => {
  const now = new Date();
  const minutes = now.getHours() * 60 + now.getMinutes();
  const start = Math.max(0, minutes - 10);
  const end = (minutes + 50) % 1440;
  const pad = (n: number) => String(n).padStart(2, '0');
  const locked = schedule({
    lock_changes: true,
    days: [now.getDay()],
    start_time: `${pad(Math.floor(start / 60))}:${pad(start % 60)}`,
    end_time: `${pad(Math.floor(end / 60))}:${pad(end % 60)}`,
  });
  const snapshot = {
    ...structuredClone(emptySnapshot),
    sessions: [templateSession],
    schedules: [locked],
  };
  useAppStore.setState({
    data: snapshot,
    loading: false,
    error: null,
    busy: false,
  });
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  expect(await screen.findByText('Morning block')).toBeInTheDocument();
  expect(screen.getByText('Locked')).toBeInTheDocument();
  expect(screen.getByText('Active')).toBeInTheDocument();
  expect(screen.getByRole('checkbox', { name: 'Enabled' })).toBeDisabled();
});

it('creates a schedule from a template and snapshots its focus', async () => {
  const snapshot = {
    ...structuredClone(emptySnapshot),
    sessions: [templateSession],
    schedules: [],
  };
  useAppStore.setState({
    data: snapshot,
    loading: false,
    error: null,
    busy: false,
  });
  vi.mocked(api.execute).mockResolvedValue({
    ...snapshot,
    schedules: [schedule({})],
  });
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  await user.click(screen.getByRole('button', { name: 'New schedule' }));
  await user.click(screen.getByLabelText('Template session'));
  await user.click(
    screen.getByRole('option', { name: 'Write outline · 25 min' }),
  );
  expect(screen.getByLabelText('Schedule name')).toHaveValue('Write outline');
  // No day selected is a validation error, not a backend call.
  for (const day of ['Mon', 'Tue', 'Wed', 'Thu', 'Fri']) {
    await user.click(screen.getByRole('checkbox', { name: day }));
  }
  await user.click(screen.getByRole('button', { name: 'Create schedule' }));
  expect(await screen.findByText('Pick at least one day.')).toBeInTheDocument();
  expect(api.execute).not.toHaveBeenCalled();
  await user.click(screen.getByRole('checkbox', { name: 'Mon' }));
  await user.click(screen.getByRole('button', { name: 'Create schedule' }));
  expect(api.execute).toHaveBeenCalledWith({
    type: 'create_schedule',
    title: 'Write outline',
    days: [1],
    start_time: '09:00',
    end_time: '10:00',
    focus: templateSession.focus,
    enabled: true,
    lock_changes: false,
  });
});

it('shows guidance when no session exists to use as a template', async () => {
  useAppStore.setState({
    data: structuredClone(emptySnapshot),
    loading: false,
    error: null,
    busy: false,
  });
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  expect(
    await screen.findByText(/Create a session template first/),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole('button', { name: 'New schedule' }),
  ).not.toBeInTheDocument();
});
