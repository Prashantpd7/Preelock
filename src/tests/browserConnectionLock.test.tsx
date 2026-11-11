import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { PermissionsPanel } from '../components/PermissionsPanel';
import { BrowserSetup } from '../components/BrowserSetup';
import { useAppStore } from '../state/store';
import { api, emptySnapshot } from '../services/api';
import { settingsAction } from '../services/settings';
import { defaultFocus, type FocusSetup, type Session } from '../types';

vi.mock('../services/api', () => ({
  desktop: true,
  emptySnapshot: {
    sessions: [],
    tasks: [],
    preferences: { theme: 'dark', default_minutes: 25 },
  },
  api: {
    focusSetup: vi.fn(),
    resetPairing: vi.fn(),
    snapshot: vi.fn(),
    openSetup: vi.fn(),
    requestCamera: vi.fn(),
  },
}));
vi.mock('../services/settings', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../services/settings')>()),
  settingsAction: vi.fn(),
}));

const connectedSetup: FocusSetup = {
  platform: 'macos',
  native_available: true,
  applications: [],
  camera_permission: 'granted',
  pairing_token: 'ABC123',
  browser: {
    available: true,
    connected: true,
    applied_id: '',
    error: '',
    blocks: 0,
    cleanup_pending: false,
    extension_stale: false,
  },
  connected_browsers: ['com.google.chrome'],
  browsers: [{ id: 'com.google.chrome', name: 'Chrome' }],
};

let seq = 0;
function makeSession(over: Partial<Session> = {}): Session {
  seq += 1;
  return {
    id: `s${seq}`,
    title: `Session ${seq}`,
    duration_minutes: 25,
    remaining_ms: 0,
    focused_ms: 0,
    allow_pause: true,
    status: 'completed',
    started_at: null,
    ended_at: null,
    deadline_ms: null,
    focus: defaultFocus,
    ...over,
  };
}
function strictSession(): Session {
  return makeSession({
    title: 'Strict work',
    status: 'running',
    started_at: Date.now(),
    focus: { ...defaultFocus, mode: 'strict' },
  });
}
function setSessions(sessions: Session[]) {
  useAppStore.setState({
    data: { ...structuredClone(emptySnapshot), sessions },
    loading: false,
    error: null,
    busy: false,
  });
}

beforeEach(() => {
  seq = 0;
  vi.clearAllMocks();
  localStorage.clear();
  HTMLDialogElement.prototype.showModal = function () {
    this.setAttribute('open', '');
  };
  HTMLDialogElement.prototype.close = function () {
    this.removeAttribute('open');
  };
  setSessions([]);
  vi.mocked(api.focusSetup).mockResolvedValue(connectedSetup);
  vi.mocked(api.resetPairing).mockResolvedValue(undefined);
  vi.mocked(settingsAction).mockResolvedValue({});
});

async function openBrowserModal() {
  const user = userEvent.setup();
  // The setup loads asynchronously; wait for the connected state.
  await user.click(await screen.findByRole('button', { name: 'Connected' }));
  expect(await screen.findByRole('dialog')).toHaveTextContent(
    'Browser connection',
  );
  return user;
}

describe('browser connection lock during a strict focus session', () => {
  it('shows the locked popup when connecting another browser', async () => {
    setSessions([strictSession()]);
    render(<PermissionsPanel />);
    const user = await openBrowserModal();
    await user.click(
      screen.getByRole('button', { name: 'Connect another browser' }),
    );
    expect(await screen.findByRole('dialog')).toHaveTextContent(
      'Browser settings locked',
    );
    expect(
      screen.getByText(/cannot be changed during an active focus session/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        /You can modify these settings after your current session ends/,
      ),
    ).toBeInTheDocument();
    // The action was not performed: no add-browser panel appears.
    expect(
      screen.queryByRole('button', { name: 'Connect another browser' }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Got it' }));
    expect(
      screen.queryByText('Browser settings locked'),
    ).not.toBeInTheDocument();
  });

  it('shows the locked popup and does not reset when resetting all connections', async () => {
    setSessions([strictSession()]);
    render(<PermissionsPanel />);
    const user = await openBrowserModal();
    await user.click(
      screen.getByRole('button', { name: 'Reset all browser connections' }),
    );
    expect(await screen.findByRole('dialog')).toHaveTextContent(
      'Browser settings locked',
    );
    expect(api.resetPairing).not.toHaveBeenCalled();
  });

  it('connect another browser works when no strict session is live', async () => {
    render(<PermissionsPanel />);
    const user = await openBrowserModal();
    await user.click(
      screen.getByRole('button', { name: 'Connect another browser' }),
    );
    expect(
      screen.queryByText('Browser settings locked'),
    ).not.toBeInTheDocument();
    expect(api.resetPairing).not.toHaveBeenCalled();
  });

  it('reset works when no strict session is live', async () => {
    render(<PermissionsPanel />);
    const user = await openBrowserModal();
    await user.click(
      screen.getByRole('button', { name: 'Reset all browser connections' }),
    );
    expect(api.resetPairing).toHaveBeenCalledTimes(1);
    expect(
      screen.queryByText('Browser settings locked'),
    ).not.toBeInTheDocument();
  });

  it('a live standard session does not lock connection settings', async () => {
    setSessions([
      makeSession({
        status: 'running',
        focus: { ...defaultFocus, mode: 'standard' },
      }),
    ]);
    render(<PermissionsPanel />);
    const user = await openBrowserModal();
    await user.click(
      screen.getByRole('button', { name: 'Reset all browser connections' }),
    );
    expect(api.resetPairing).toHaveBeenCalledTimes(1);
    expect(
      screen.queryByText('Browser settings locked'),
    ).not.toBeInTheDocument();
  });

  it('settings unlock after the strict session ends', async () => {
    setSessions([
      makeSession({
        status: 'completed',
        focus: { ...defaultFocus, mode: 'strict' },
      }),
    ]);
    render(<PermissionsPanel />);
    const user = await openBrowserModal();
    await user.click(
      screen.getByRole('button', { name: 'Reset all browser connections' }),
    );
    expect(api.resetPairing).toHaveBeenCalledTimes(1);
    expect(
      screen.queryByText('Browser settings locked'),
    ).not.toBeInTheDocument();
  });

  it('BrowserSetup disconnect link is locked during a strict session', async () => {
    setSessions([strictSession()]);
    render(<BrowserSetup setup={connectedSetup} refresh={vi.fn()} />);
    const user = userEvent.setup();
    await user.click(
      screen.getByRole('button', { name: 'Disconnect and reconnect' }),
    );
    expect(await screen.findByRole('dialog')).toHaveTextContent(
      'Browser settings locked',
    );
    expect(api.resetPairing).not.toHaveBeenCalled();
  });

  it('BrowserSetup disconnect link works after the session ends', async () => {
    render(<BrowserSetup setup={connectedSetup} refresh={vi.fn()} />);
    const user = userEvent.setup();
    await user.click(
      screen.getByRole('button', { name: 'Disconnect and reconnect' }),
    );
    expect(api.resetPairing).toHaveBeenCalledTimes(1);
    expect(
      screen.queryByText('Browser settings locked'),
    ).not.toBeInTheDocument();
  });
});
