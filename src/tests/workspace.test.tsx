import { beforeEach, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { App } from '../App';
import { api, emptySnapshot } from '../services/api';
import { useAppStore } from '../state/store';
import { defaultFocus } from '../types';
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
  },
}));
beforeEach(() => {
  localStorage.setItem('vystriq-permissions-v2', 'seen');
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
    'Activity',
    'Analytics',
    'Settings',
  ]) {
    await user.click(screen.getByRole('link', { name: label }));
    expect(
      await screen.findByRole('heading', { name: label }),
    ).toBeInTheDocument();
  }
  await user.click(screen.getByRole('link', { name: 'Tasks' }));
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
  vi.mocked(api.execute).mockResolvedValue(emptySnapshot);
  await user.click(screen.getByRole('button', { name: 'Create session' }));
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
      general_internet: true,
      browser_app: '',
      activity_consent: true,
      camera: false,
      attention: {
        ...defaultFocus.attention,
        verification: 'activity_only',
        work_apps: ['com.microsoft.VSCode'],
      },
    },
  });
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
  expect(screen.getByLabelText('Which browser?')).toHaveValue(
    'ai.perplexity.comet',
  );
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
    screen.getByRole('button', { name: 'Open Extensions' }),
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
