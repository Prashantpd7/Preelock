import { beforeEach, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { App } from '../App';
import { PlanningSections } from '../components/PlanningSections';
import { api, emptySnapshot } from '../services/api';
import { useAppStore } from '../state/store';
import { defaultFocus, type Schedule, type Session } from '../types';
import {
  formatDays,
  formatScheduleDuration,
  isScheduleActive,
  scheduleTotalTime,
  scheduleWindow,
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

it('computes schedule window durations, including overnight', () => {
  expect(scheduleWindow('09:00', '10:00')).toEqual({
    minutes: 60,
    overnight: false,
  });
  expect(scheduleWindow('09:00', '11:30')).toEqual({
    minutes: 150,
    overnight: false,
  });
  expect(scheduleWindow('21:00', '06:00')).toEqual({
    minutes: 540,
    overnight: true,
  });
  // End equal to start follows the overnight rule: a full day.
  expect(scheduleWindow('09:00', '09:00')).toEqual({
    minutes: 1440,
    overnight: true,
  });
  expect(scheduleWindow('25:00', '10:00')).toBeNull();
  expect(formatScheduleDuration(45)).toBe('45 min');
  expect(formatScheduleDuration(60)).toBe('1 hr');
  expect(formatScheduleDuration(90)).toBe('1 hr 30 min');
  expect(scheduleTotalTime('09:00', '10:00')).toBe('1 hr');
  expect(scheduleTotalTime('09:00', '11:30')).toBe('2 hr 30 min');
  expect(scheduleTotalTime('09:00', '09:45')).toBe('45 min');
  expect(scheduleTotalTime('21:00', '06:00')).toBe('9 hr · Overnight');
  expect(scheduleTotalTime('09:00', '09:00')).toBe('24 hr · Overnight');
  expect(scheduleTotalTime('25:00', '10:00')).toBe('—');
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

it('creates a schedule from the wizard and snapshots its focus', async () => {
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
  const setup = await api.focusSetup();
  vi.mocked(api.focusSetup).mockResolvedValue({
    ...setup,
    applications: [{ id: 'com.microsoft.VSCode', name: 'VS Code' }],
  });
  vi.mocked(api.execute).mockResolvedValue({
    ...snapshot,
    schedules: [schedule({})],
  });
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  await user.click(screen.getByRole('button', { name: 'New schedule' }));
  // The wizard opens — same steps as starting a session.
  await user.type(screen.getByLabelText('Task name'), 'Write outline');
  await user.click(screen.getByRole('radio', { name: /Apps only/ }));
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  await user.click(screen.getByRole('checkbox', { name: 'VS Code' }));
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  // Schedule mode ends in the schedule popup, not "Start session?".
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  expect(
    await screen.findByRole('heading', { name: 'Schedule session' }),
  ).toBeInTheDocument();
  expect(screen.getByLabelText('Schedule name')).toHaveValue('Write outline');
  expect(
    screen.queryByRole('button', { name: 'Start session' }),
  ).not.toBeInTheDocument();
  // No day selected is a validation error, not a backend call.
  for (const day of ['Mon', 'Tue', 'Wed', 'Thu', 'Fri']) {
    await user.click(screen.getByRole('checkbox', { name: day }));
  }
  await user.click(screen.getByRole('button', { name: 'Create schedule' }));
  expect(await screen.findByText('Pick at least one day.')).toBeInTheDocument();
  expect(api.execute).not.toHaveBeenCalled();
  await user.click(screen.getByRole('checkbox', { name: 'Mon' }));
  await user.click(screen.getByRole('button', { name: 'Create schedule' }));
  expect(api.execute).toHaveBeenCalledWith(
    expect.objectContaining({
      type: 'create_schedule',
      title: 'Write outline',
      days: [1],
      start_time: '09:00',
      end_time: '10:00',
      enabled: true,
      lock_changes: false,
    }),
  );
  const call = vi.mocked(api.execute).mock.calls[0][0];
  expect(call.type).toBe('create_schedule');
  if (call.type === 'create_schedule') {
    expect(call.focus.attention?.work_apps).toEqual(['com.microsoft.VSCode']);
  }
});

it('shows the New schedule button even with no sessions, and it opens the wizard', async () => {
  useAppStore.setState({
    data: structuredClone(emptySnapshot),
    loading: false,
    error: null,
    busy: false,
  });
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  expect(
    screen.queryByText(/Create a session template first/),
  ).not.toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'New schedule' }));
  expect(await screen.findByLabelText('Task name')).toBeInTheDocument();
});

it('still opens the template form when rendered without the wizard callback', async () => {
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
  render(<PlanningSections only="schedules" />);
  await user.click(screen.getByRole('button', { name: 'New schedule' }));
  await user.click(screen.getByLabelText('Template session'));
  await user.click(
    screen.getByRole('option', { name: 'Write outline · 25 min' }),
  );
  expect(screen.getByLabelText('Schedule name')).toHaveValue('Write outline');
});

it('edit form offers only the password lock and clears stored delay/restart locks on save', async () => {
  // A schedule saved when the other lock kinds existed: delay +
  // restart locks on, password lock on. Its window is not active
  // (a different day), so the edit form opens without proofs.
  const otherDay = (new Date().getDay() + 1) % 7;
  const stored = schedule({
    days: [otherDay],
    lock: {
      password_hash: 'v1:100000:aa:bb',
      challenge_length: 0,
      delay_minutes: 5,
      restart_lock: true,
    },
  });
  const snapshot = {
    ...structuredClone(emptySnapshot),
    sessions: [templateSession],
    schedules: [stored],
  };
  useAppStore.setState({
    data: snapshot,
    loading: false,
    error: null,
    busy: false,
  });
  vi.mocked(api.execute).mockResolvedValue({ ...snapshot });
  const user = userEvent.setup();
  render(<PlanningSections only="schedules" />);
  expect(await screen.findByText('Morning block')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Edit' }));
  expect(
    await screen.findByRole('heading', { name: 'Edit schedule' }),
  ).toBeInTheDocument();
  expect(screen.getByLabelText('Total time')).toHaveValue('1 hr');
  expect(
    screen.getByRole('checkbox', { name: 'Lock with a password' }),
  ).toBeChecked();
  for (const removed of [
    'Lock changes during the window',
    'Lock with a typed challenge',
    'Lock with a delay',
    'Keep the lock after a restart',
  ]) {
    expect(
      screen.queryByRole('checkbox', { name: removed }),
    ).not.toBeInTheDocument();
  }
  await user.click(screen.getByRole('button', { name: 'Save schedule' }));
  expect(api.execute).toHaveBeenCalledWith(
    expect.objectContaining({
      type: 'update_schedule',
      id: 'sched-1',
      lock_changes: false,
    }),
  );
  const call = vi.mocked(api.execute).mock.calls[0][0];
  expect(call.type).toBe('update_schedule');
  if (call.type === 'update_schedule') {
    // The stored delay and restart locks are gone; the password
    // hash is kept (the field was left empty on purpose).
    expect(call.lock).toEqual({
      password_hash: 'v1:100000:aa:bb',
      challenge_length: 0,
      delay_minutes: 0,
      restart_lock: false,
    });
    expect(call.lock_password).toBeUndefined();
  }
});
