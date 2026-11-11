import { beforeEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { App } from '../App';
import { PlanningSections } from '../components/PlanningSections';
import { api, emptySnapshot } from '../services/api';
import { useAppStore } from '../state/store';
import { defaultFocus, type OneShot, type Session } from '../types';
import { oneShotFireAtMs, validateOneShotForm } from '../services/one_shots';

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
    one_shots: [],
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

function oneShot(overrides: Partial<OneShot>): OneShot {
  return {
    id: 'one-1',
    title: 'Write outline',
    minutes: 25,
    allow_pause: true,
    focus: structuredClone(defaultFocus),
    fire_at_ms: Date.now() + 3_600_000,
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

it('validates the one-shot form: template, title and a future time', () => {
  const base = {
    title: 'Write outline',
    templateId: 'sess-1',
    fireAtMs: 2_000_000,
    nowMs: 1_000_000,
  };
  expect(validateOneShotForm(base)).toBe('');
  expect(validateOneShotForm({ ...base, templateId: '' })).toContain(
    'template',
  );
  expect(validateOneShotForm({ ...base, title: '  ' })).toContain('title');
  expect(validateOneShotForm({ ...base, fireAtMs: null })).toContain(
    'date and time',
  );
  expect(validateOneShotForm({ ...base, fireAtMs: 1_000_000 })).toContain(
    'future',
  );
  expect(validateOneShotForm({ ...base, fireAtMs: 999_999 })).toContain(
    'future',
  );
  expect(oneShotFireAtMs('')).toBeNull();
  expect(oneShotFireAtMs('not-a-date')).toBeNull();
});

it('explains that an enabled schedule turns on open at login', async () => {
  useAppStore.setState({
    data: {
      ...structuredClone(emptySnapshot),
      sessions: [templateSession],
    },
    loading: false,
    error: null,
    busy: false,
  });
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  expect(
    await screen.findByText(/Needs open at login to run/),
  ).toBeInTheDocument();
});

it('lists pending one-shots and cancels one', async () => {
  const pending = oneShot({});
  const snapshot = {
    ...structuredClone(emptySnapshot),
    sessions: [templateSession],
    one_shots: [pending],
  };
  useAppStore.setState({
    data: snapshot,
    loading: false,
    error: null,
    busy: false,
  });
  vi.mocked(api.execute).mockResolvedValue({
    ...snapshot,
    one_shots: [],
  });
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  // The one-shot section is auto-selected: a future timer exists, so
  // the dropdown shows it and the pending timer is listed below.
  expect(
    await screen.findByRole('button', { name: 'Start once at' }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole('button', { name: 'Cancel timer' }),
  ).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Cancel timer' }));
  expect(api.execute).toHaveBeenCalledWith({
    type: 'delete_one_shot',
    id: 'one-1',
  });
});

it('creates a one-shot from the wizard at a future time', async () => {
  const snapshot = {
    ...structuredClone(emptySnapshot),
    sessions: [templateSession],
    one_shots: [],
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
    one_shots: [oneShot({})],
  });
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  await user.click(screen.getByRole('button', { name: 'New one-shot' }));
  // The wizard opens — same steps as starting a session.
  await user.type(screen.getByLabelText('Task name'), 'Write outline');
  await user.click(screen.getByRole('radio', { name: /Apps only/ }));
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  await user.click(screen.getByRole('checkbox', { name: 'VS Code' }));
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  // One-shot mode ends in the timing popup, not "Start session?".
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  expect(
    await screen.findByRole('heading', { name: 'Start once at' }),
  ).toBeInTheDocument();
  expect(screen.getByLabelText('One-shot name')).toHaveValue('Write outline');
  expect(
    screen.queryByRole('button', { name: 'Start session' }),
  ).not.toBeInTheDocument();
  // A past time is a validation error, not a backend call.
  fireEvent.change(screen.getByLabelText('Start at'), {
    target: { value: '2020-01-01T09:00' },
  });
  await user.click(screen.getByRole('button', { name: 'Create one-shot' }));
  expect(
    await screen.findByText('Pick a future date and time.'),
  ).toBeInTheDocument();
  expect(api.execute).not.toHaveBeenCalled();
  fireEvent.change(screen.getByLabelText('Start at'), {
    target: { value: '2099-01-01T09:00' },
  });
  await user.click(screen.getByRole('button', { name: 'Create one-shot' }));
  expect(api.execute).toHaveBeenCalledWith(
    expect.objectContaining({
      type: 'create_one_shot',
      title: 'Write outline',
      minutes: 25,
      allow_pause: true,
      fire_at_ms: new Date('2099-01-01T09:00').getTime(),
    }),
  );
  const call = vi.mocked(api.execute).mock.calls[0][0];
  expect(call.type).toBe('create_one_shot');
  if (call.type === 'create_one_shot') {
    expect(call.focus.attention?.work_apps).toEqual(['com.microsoft.VSCode']);
  }
});

it('shows the New one-shot button even with no sessions, and it opens the wizard', async () => {
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
    screen.queryByText(/No session templates yet/),
  ).not.toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'New one-shot' }));
  expect(await screen.findByLabelText('Task name')).toBeInTheDocument();
});

it('still opens the template form when rendered without the wizard callback', async () => {
  const snapshot = {
    ...structuredClone(emptySnapshot),
    sessions: [templateSession],
    one_shots: [],
  };
  useAppStore.setState({
    data: snapshot,
    loading: false,
    error: null,
    busy: false,
  });
  vi.mocked(api.execute).mockResolvedValue({
    ...snapshot,
    one_shots: [oneShot({})],
  });
  const user = userEvent.setup();
  render(<PlanningSections only="oneshots" />);
  await user.click(screen.getByRole('button', { name: 'New one-shot' }));
  await user.click(screen.getByLabelText('One-shot template'));
  await user.click(
    screen.getByRole('option', { name: 'Write outline · 25 min' }),
  );
  expect(screen.getByLabelText('One-shot name')).toHaveValue('Write outline');
});
