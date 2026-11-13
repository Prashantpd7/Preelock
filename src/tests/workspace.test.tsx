import { beforeEach, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { App } from '../App';
import { api, emptySnapshot } from '../services/api';
import { useAppStore } from '../state/store';
import { defaultFocus, type SavedSetup, type Session } from '../types';
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
beforeEach(() => {
  localStorage.setItem('preelock-permissions-v2', 'seen');
  window.location.hash = '#/';
  vi.clearAllMocks();
  useAppStore.setState({
    data: structuredClone(emptySnapshot),
    loading: false,
    error: null,
    busy: false,
  });
  vi.mocked(api.snapshot).mockResolvedValue(structuredClone(emptySnapshot));
  vi.mocked(api.capabilities).mockResolvedValue([]);
  vi.mocked(api.focusSetup).mockResolvedValue({
    platform: 'macos',
    native_available: true,
    applications: [],
    camera_permission: 'not_requested',
    pairing_token: '',
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
it('navigates all six routes and submits a task to the native boundary', async () => {
  const user = userEvent.setup();
  render(<App />);
  expect(
    await screen.findByRole('heading', { name: 'Make room for deep work.' }),
  ).toBeInTheDocument();
  for (const label of [
    'Focus Sessions',
    'Tasks',
    'Analytics',
    'Screen Time',
    'Settings',
  ]) {
    await user.click(screen.getByRole('link', { name: label }));
    expect(
      await screen.findByRole('heading', { name: label }),
    ).toBeInTheDocument();
  }
  await user.click(screen.getByRole('link', { name: 'Tasks' }));
  vi.mocked(api.snapshot).mockImplementation(async () =>
    structuredClone(useAppStore.getState().data),
  );
  vi.mocked(api.execute).mockResolvedValue({
    ...emptySnapshot,
    tasks: [
      { id: 't', title: 'Read chapter one', completed: false, created_at: 1 },
    ],
  });
  await user.type(
    screen.getByRole('textbox', { name: 'New task' }),
    'Read chapter one',
  );
  await user.click(screen.getByRole('button', { name: 'Add task' }));
  expect(api.execute).toHaveBeenCalledWith({
    type: 'create_task',
    title: 'Read chapter one',
  });
  expect(await screen.findByText('Read chapter one')).toBeInTheDocument();
});
it('creates a flexible session with app checks', async () => {
  const setup = await api.focusSetup();
  vi.mocked(api.focusSetup).mockResolvedValue({
    ...setup,
    applications: [{ id: 'com.microsoft.VSCode', name: 'VS Code' }],
  });
  const user = userEvent.setup();
  window.location.hash = '#/sessions';
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  await user.click(screen.getByRole('button', { name: 'New session' }));
  await user.type(screen.getByLabelText('Task name'), 'Write outline');
  await user.click(screen.getByRole('radio', { name: /Apps only/ }));
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  await user.click(screen.getByRole('checkbox', { name: 'VS Code' }));
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  const createdSession: Session = {
    id: 'new1',
    title: 'Write outline',
    duration_minutes: 25,
    remaining_ms: 0,
    focused_ms: 0,
    allow_pause: true,
    status: 'ready',
    started_at: null,
    ended_at: null,
    deadline_ms: null,
    focus: defaultFocus,
  };
  vi.mocked(api.execute).mockImplementation(async (action) =>
    action.type === 'create_session'
      ? { ...emptySnapshot, sessions: [createdSession] }
      : emptySnapshot,
  );
  await user.click(screen.getByRole('button', { name: 'Create session' }));
  // Confirmation popup shows the review before anything is created.
  expect(
    await screen.findByRole('button', { name: 'Start session' }),
  ).toBeInTheDocument();
  expect(screen.getByText('Write outline')).toBeInTheDocument();
  expect(screen.getByText('25 minutes')).toBeInTheDocument();
  expect(screen.getByText('Standard')).toBeInTheDocument();
  expect(api.execute).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: 'Start session' }));
  expect(api.execute).toHaveBeenCalledWith({
    type: 'create_session',
    title: 'Write outline',
    minutes: 25,
    allow_pause: true,
    focus: {
      mode: 'standard',
      website_blocking: 'none',
      apps: [],
      domains: [],
      exceptions: [],
      blocked_folders: [],
      blocked_titles: [],
      block_adult_content: false,
      youtube_blocked_channels: [],
      youtube_allowed_channels: [],
      youtube_study_mode: false,
      youtube_block_entertainment: false,
      youtube_allow_watch_later: false,
      general_internet: true,
      browser_app: '',
      activity_consent: true,
      camera: false,
      attention: {
        ...defaultFocus.attention,
        verification: 'activity_only',
        work_apps: ['com.microsoft.VSCode'],
      },
      // Step 7: the wizard submits the Pomodoro config (disabled by
      // default) as part of the focus snapshot.
      pomodoro: {
        ...defaultFocus.pomodoro,
      },
      // Step 9: same for the reward config (disabled by default).
      reward: {
        ...defaultFocus.reward,
      },
      // Step 12: same for the frozen config (disabled by default).
      frozen: {
        enabled: false,
        allowed_apps: [],
        allowed_websites: [],
      },
    },
  });
  // "Start session" starts the freshly created session immediately.
  await waitFor(() =>
    expect(api.execute).toHaveBeenCalledWith({
      type: 'start_session',
      id: 'new1',
    }),
  );
});
it('renames the start card, drops the block-other-apps choice, and still blocks other apps', async () => {
  const setup = await api.focusSetup();
  vi.mocked(api.focusSetup).mockResolvedValue({
    ...setup,
    applications: [{ id: 'com.microsoft.VSCode', name: 'VS Code' }],
  });
  const user = userEvent.setup();
  window.location.hash = '#/sessions';
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  // Item 2: the first card is now the instant session card.
  expect(
    screen.getByRole('heading', { name: 'Start instant focus session' }),
  ).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'New session' }));
  await user.type(screen.getByLabelText('Task name'), 'Write outline');
  await user.click(screen.getByRole('radio', { name: /Apps only/ }));
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  await user.click(screen.getByRole('checkbox', { name: 'VS Code' }));
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  // Item 1: the Ready step no longer offers the choice at all.
  expect(
    screen.queryByRole('checkbox', { name: /Block all other apps/i }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByText('Block all other apps during this session'),
  ).not.toBeInTheDocument();
  const createdSession: Session = {
    id: 'new1',
    title: 'Write outline',
    duration_minutes: 25,
    remaining_ms: 0,
    focused_ms: 0,
    allow_pause: true,
    status: 'ready',
    started_at: null,
    ended_at: null,
    deadline_ms: null,
    focus: defaultFocus,
  };
  vi.mocked(api.execute).mockImplementation(async (action) =>
    action.type === 'create_session'
      ? { ...emptySnapshot, sessions: [createdSession] }
      : emptySnapshot,
  );
  await user.click(screen.getByRole('button', { name: 'Create session' }));
  expect(
    await screen.findByRole('button', { name: 'Start session' }),
  ).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Start session' }));
  // Item 6: a Flexible wizard session carries allow_pause through,
  // and the payload still blocks other apps as if the old default
  // (checked) were set.
  expect(api.execute).toHaveBeenCalledWith(
    expect.objectContaining({
      type: 'create_session',
      allow_pause: true,
      focus: expect.objectContaining({ activity_consent: true }),
    }),
  );
});
it('drops the distraction-grace choice and still submits the 10-second grace', async () => {
  const setup = await api.focusSetup();
  vi.mocked(api.focusSetup).mockResolvedValue({
    ...setup,
    applications: [{ id: 'com.microsoft.VSCode', name: 'VS Code' }],
  });
  // A setup saved when the choice existed can carry a different grace.
  const preset: SavedSetup = {
    id: 'setup1',
    name: 'Old setup',
    minutes: 25,
    focus: {
      ...defaultFocus,
      attention: {
        enabled: true,
        task: 'other',
        verification: 'activity_only',
        grace_seconds: 20,
        camera_fallback: 'unknown',
        work_apps: ['com.microsoft.VSCode'],
        work_domains: [],
      },
    },
  };
  const seeded = {
    ...structuredClone(emptySnapshot),
    preferences: {
      ...structuredClone(emptySnapshot).preferences,
      saved_setups: [preset],
    },
  };
  vi.mocked(api.snapshot).mockResolvedValue(seeded);
  useAppStore.setState({
    data: seeded,
    loading: false,
    error: null,
    busy: false,
  });
  const user = userEvent.setup();
  window.location.hash = '#/sessions';
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  // Applying the saved setup opens the wizard with its values.
  await user.click(screen.getByRole('button', { name: /Old setup/ }));
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  // Ready step: the distraction-grace choice is gone.
  expect(
    screen.queryByText('Wait before labeling a distraction'),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByText('10 seconds (recommended)'),
  ).not.toBeInTheDocument();
  const createdSession: Session = {
    id: 'new1',
    title: 'Old setup',
    duration_minutes: 25,
    remaining_ms: 0,
    focused_ms: 0,
    allow_pause: true,
    status: 'ready',
    started_at: null,
    ended_at: null,
    deadline_ms: null,
    focus: defaultFocus,
  };
  vi.mocked(api.execute).mockImplementation(async (action) =>
    action.type === 'create_session'
      ? { ...emptySnapshot, sessions: [createdSession] }
      : emptySnapshot,
  );
  await user.click(screen.getByRole('button', { name: 'Create session' }));
  expect(
    await screen.findByRole('button', { name: 'Start session' }),
  ).toBeInTheDocument();
  // The confirmation review no longer shows the grace either.
  expect(
    screen.queryByText(/Wait before labeling a distraction/),
  ).not.toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Start session' }));
  // The submitted payload still carries the old default of 10 seconds,
  // even though the saved setup stored 20.
  expect(api.execute).toHaveBeenCalledWith(
    expect.objectContaining({
      type: 'create_session',
      focus: expect.objectContaining({
        attention: expect.objectContaining({ grace_seconds: 10 }),
      }),
    }),
  );
});
it('reports persistence errors without inventing success', async () => {
  vi.mocked(api.execute).mockRejectedValue(new Error('Disk is full.'));
  await useAppStore.getState().execute({ type: 'create_task', title: 'Read' });
  await waitFor(() =>
    expect(useAppStore.getState().error).toBe('Disk is full.'),
  );
  expect(useAppStore.getState().data.tasks).toHaveLength(0);
});

it('shows Comet and stops unpaired website setup before session creation', async () => {
  const setup = await api.focusSetup();
  vi.mocked(api.focusSetup).mockResolvedValue({
    ...setup,
    applications: [{ id: 'ai.perplexity.comet', name: 'Comet' }],
    browsers: [{ id: 'ai.perplexity.comet', name: 'Comet' }],
  });
  const user = userEvent.setup();
  window.location.hash = '#/sessions';
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  await user.click(screen.getByRole('button', { name: 'New session' }));
  await user.type(screen.getByLabelText('Task name'), 'Read docs');
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  await user.click(screen.getByRole('checkbox', { name: 'Comet' }));
  await user.click(screen.getByRole('checkbox', { name: 'Website blocking' }));
  await user.click(
    screen.getByRole('checkbox', { name: 'Block all websites except these' }),
  );
  await user.type(screen.getByLabelText('Websites to keep open'), 'docs.rs');
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  expect(
    await screen.findByText(
      'Connect the browser extension below, then check again.',
    ),
  ).toHaveAttribute('role', 'alert');
  expect(
    screen.getByRole('button', { name: 'Add to Comet' }),
  ).toBeInTheDocument();
  expect(api.execute).not.toHaveBeenCalled();
});
it('keeps the removed sidebar section empty', async () => {
  render(<App />);
  await screen.findByRole('heading', { name: 'Make room for deep work.' });
  expect(screen.queryByText('Your focus. Your space.')).not.toBeInTheDocument();
  expect(screen.queryByText('Privacy & capabilities')).not.toBeInTheDocument();
});

it('disables camera checking on Windows', async () => {
  const setup = await api.focusSetup();
  vi.mocked(api.focusSetup).mockResolvedValue({
    ...setup,
    platform: 'windows',
    camera_permission: 'unsupported',
  });
  const user = userEvent.setup();
  window.location.hash = '#/sessions';
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  await user.click(screen.getByRole('button', { name: 'New session' }));
  expect(screen.getByRole('radio', { name: /Apps \+ camera/ })).toBeDisabled();
  expect(screen.getByRole('radio', { name: /Apps only/ })).toBeChecked();
});
