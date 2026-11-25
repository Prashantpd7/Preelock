import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { SessionCard } from '../components/SessionCard';
import { useAppStore } from '../state/store';
import { api } from '../services/api';
import { defaultFocus, type FocusSetup, type Session } from '../types';

vi.mock('../services/api', () => ({
  desktop: true,
  emptySnapshot: {
    sessions: [],
    tasks: [],
    schedules: [],
    one_shots: [],
    allowances: [],
    preferences: { theme: 'dark', default_minutes: 25 },
  },
  api: {
    focusSetup: vi.fn(),
    resetPairing: vi.fn(),
    snapshot: vi.fn(),
    openSetup: vi.fn(),
    requestCamera: vi.fn(),
    blockAttempts: vi.fn(),
    appIcon: vi.fn(),
    exportStatistics: vi.fn(),
    deleteStatistics: vi.fn(),
    capabilities: vi.fn(),
    dismissBlockedAppPage: vi.fn(),
    execute: vi.fn(),
  },
}));

const baseSetup: FocusSetup = {
  platform: 'macos',
  native_available: true,
  applications: [{ id: 'com.example.focus-app', name: 'Focus App' }],
  camera_permission: 'granted',
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
  connected_browsers: [],
  browsers: [{ id: 'com.google.chrome', name: 'Chrome' }],
};

const session: Session = {
  id: 's1',
  title: 'Deep work',
  duration_minutes: 25,
  remaining_ms: 25 * 60_000,
  focused_ms: 0,
  allow_pause: true,
  status: 'ready',
  started_at: null,
  ended_at: null,
  deadline_ms: null,
  focus: {
    ...structuredClone(defaultFocus),
    apps: ['com.example.focus-app'],
    domains: ['youtube.com'],
    website_blocking: 'blocklist',
    youtube_blocked_channels: ['@mkbhd'],
    browser_app: 'com.google.Chrome',
  },
};

beforeEach(() => {
  vi.mocked(api.focusSetup).mockResolvedValue(structuredClone(baseSetup));
  useAppStore.setState({
    data: {
      sessions: [session],
      tasks: [],
      schedules: [],
      one_shots: [],
      allowances: [],
      preferences: { theme: 'dark', default_minutes: 25 },
    },
    loading: false,
    error: null,
    busy: false,
  });
});

describe('SessionCard Before you start review', () => {
  it('starts a saved session directly without requiring browser connection', async () => {
    const user = userEvent.setup();
    const executeSpy = vi.fn().mockResolvedValue(true);
    useAppStore.setState({ execute: executeSpy });
    render(
      <SessionCard
        session={session}
        setup={baseSetup}
        appNames={{ 'com.example.focus-app': 'Focus App' }}
      />,
    );
    await user.click(screen.getByRole('button', { name: 'Start session' }));
    expect(executeSpy).toHaveBeenCalledWith({
      type: 'start_session',
      id: 's1',
    });
    expect(
      screen.queryByRole('heading', { name: 'Before you start' }),
    ).not.toBeInTheDocument();
  });

  it('starts YouTube-channel-only sessions on Windows without requiring browser or camera popup', async () => {
    const ytAndCamSession = {
      ...session,
      focus: {
        ...structuredClone(defaultFocus),
        apps: ['Focus.exe'],
        domains: [],
        website_blocking: 'blocklist' as const,
        youtube_blocked_channels: ['@mkbhd'],
        browser_app: 'chrome.exe',
        camera: true,
        attention: {
          ...defaultFocus.attention!,
          verification: 'camera_and_activity' as const,
        },
      },
    };
    const windowsSetup = {
      ...baseSetup,
      platform: 'windows',
      camera_permission: 'not_requested',
      browsers: [{ id: 'chrome.exe', name: 'Chrome' }],
    };
    vi.mocked(api.focusSetup).mockResolvedValue(windowsSetup);
    const executeSpy = vi.fn().mockResolvedValue(true);
    useAppStore.setState({ execute: executeSpy });
    const user = userEvent.setup();
    render(
      <SessionCard
        session={ytAndCamSession}
        setup={windowsSetup}
        appNames={{ 'Focus.exe': 'Focus' }}
      />,
    );
    await user.click(screen.getByRole('button', { name: 'Start session' }));
    expect(executeSpy).toHaveBeenCalledWith({
      type: 'start_session',
      id: 's1',
    });
    expect(
      screen.queryByRole('heading', { name: 'Before you start' }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/Camera permission/)).not.toBeInTheDocument();
  });
});
