import { beforeEach, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { SessionCard } from '../components/SessionCard';
import { useAppStore } from '../state/store';
import { api } from '../services/api';
import { defaultFocus, type FocusSetup, type Session } from '../types';
vi.mock('../services/api', () => ({
  desktop: true,
  emptySnapshot: { sessions: [], tasks: [], preferences: {} },
  api: { openSetup: vi.fn(), requestCamera: vi.fn() },
}));
const execute = vi.fn().mockResolvedValue(true);
beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState({ execute, busy: false });
});
function session(camera: boolean): Session {
  return {
    id: 'recovery',
    title: 'Recovered work',
    duration_minutes: 25,
    remaining_ms: 10000,
    focused_ms: 0,
    allow_pause: true,
    status: 'running',
    started_at: 1,
    ended_at: null,
    deadline_ms: null,
    focus: {
      ...defaultFocus,
      camera,
      attention: {
        ...defaultFocus.attention!,
        verification: 'camera_and_activity',
      },
    },
    report: {
      events: [],
      approved_ms: 0,
      distracting_ms: 0,
      idle_ms: 0,
      interruptions: 0,
      active_app: '',
      enforcement: '',
      camera_status: 'camera_unavailable',
      browser_blocks: 0,
    },
  };
}
const setup = (permission: string): FocusSetup => ({
  platform: 'macos',
  native_available: true,
  applications: [],
  pairing_token: '',
  camera_permission: permission,
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
it('offers retry for a failed camera without requiring a camera-disabled audit event', async () => {
  const user = userEvent.setup();
  render(<SessionCard session={session(true)} setup={setup('granted')} />);
  await user.click(screen.getByRole('heading', { name: 'Recovered work' }));
  await user.click(screen.getByRole('button', { name: 'Retry camera' }));
  expect(execute).toHaveBeenCalledWith({
    type: 'enable_camera',
    id: 'recovery',
  });
  expect(api.requestCamera).not.toHaveBeenCalled();
});
it('offers settings for revoked permission and start for an old disabled camera', async () => {
  const user = userEvent.setup();
  const view = render(
    <SessionCard session={session(false)} setup={setup('denied')} />,
  );
  await user.click(screen.getByRole('heading', { name: 'Recovered work' }));
  await user.click(
    screen.getByRole('button', { name: 'Open camera settings' }),
  );
  expect(api.openSetup).toHaveBeenCalledWith('camera_settings');
  view.rerender(
    <SessionCard session={session(false)} setup={setup('granted')} />,
  );
  await user.click(screen.getByRole('button', { name: 'Start camera' }));
  expect(execute).toHaveBeenCalledWith({
    type: 'enable_camera',
    id: 'recovery',
  });
});
