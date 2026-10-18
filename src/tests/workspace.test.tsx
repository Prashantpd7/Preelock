import { beforeEach, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { App } from '../App';
import { api, emptySnapshot } from '../services/api';
import { useAppStore } from '../state/store';
vi.mock('../services/api', () => ({
  desktop: true,
  emptySnapshot: {
    sessions: [],
    tasks: [],
    preferences: { theme: 'dark', default_minutes: 25 },
  },
  api: { snapshot: vi.fn(), execute: vi.fn(), capabilities: vi.fn() },
}));
beforeEach(() => {
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
it('creates a session with its selected pause policy', async () => {
  const user = userEvent.setup();
  window.location.hash = '#/sessions';
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  await user.type(screen.getByLabelText('Your intention'), 'Write outline');
  await user.click(screen.getByLabelText('Allow pausing'));
  vi.mocked(api.execute).mockResolvedValue(emptySnapshot);
  await user.click(screen.getByRole('button', { name: 'Create session' }));
  expect(api.execute).toHaveBeenCalledWith({
    type: 'create_session',
    title: 'Write outline',
    minutes: 25,
    allow_pause: false,
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
