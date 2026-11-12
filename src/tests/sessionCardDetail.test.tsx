import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { SessionCard } from '../components/SessionCard';
import { useAppStore } from '../state/store';
import { api } from '../services/api';
import {
  defaultFocus,
  type FocusConfig,
  type FocusSetup,
  type Session,
} from '../types';

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

let seq = 0;
function pastSession(
  over: Partial<Session> = {},
  focusOver: Partial<FocusConfig> = {},
): Session {
  seq += 1;
  return {
    id: `past-${seq}`,
    title: 'Deep work',
    duration_minutes: 25,
    remaining_ms: 0,
    focused_ms: 3_600_000,
    allow_pause: true,
    status: 'completed',
    started_at: 1_700_000_000_000,
    ended_at: 1_700_001_500_000,
    deadline_ms: null,
    kind: 'one_shot',
    focus: {
      ...structuredClone(defaultFocus),
      domains: ['instagram.com'],
      website_blocking: 'blocklist',
      youtube_blocked_channels: ['@mkbhd', '@veritasium'],
      ...focusOver,
    },
    attention: {
      state: 'COMPLETED',
      reason: '',
      elapsed_ms: 4_200_000,
      distracted_ms: 360_000,
      absent_ms: 240_000,
      unknown_ms: 0,
      corrected_ms: 0,
      intervals: [],
    },
    ...over,
  };
}

function setData(session: Session) {
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
}

async function expand(session: Session) {
  const user = userEvent.setup();
  setData(session);
  render(<SessionCard session={session} setup={baseSetup} />);
  await user.click(screen.getByRole('button', { expanded: false }));
  expect(
    await screen.findByRole('heading', { name: 'About this session' }),
  ).toBeInTheDocument();
  return user;
}

beforeEach(() => {
  vi.mocked(api.focusSetup).mockResolvedValue(structuredClone(baseSetup));
});

describe('SessionCard past-session detail', () => {
  it('shows the type, blocked websites, YouTube channels, and focus totals', async () => {
    await expand(pastSession());
    expect(screen.getByText('One-shot session')).toBeInTheDocument();
    expect(screen.getByText('Blocked: instagram.com')).toBeInTheDocument();
    expect(
      screen.getByText('Blocked: @mkbhd, @veritasium'),
    ).toBeInTheDocument();
    expect(screen.getByText('Final result')).toBeInTheDocument();
    expect(screen.getByText('Completed the full session.')).toBeInTheDocument();
    expect(screen.getByText('Focused 60:00')).toBeInTheDocument();
    expect(screen.getByText('Distracted 06:00')).toBeInTheDocument();
    expect(screen.getByText('Away 04:00')).toBeInTheDocument();
    expect(screen.getByText('Total 70:00')).toBeInTheDocument();
  });

  it('shows a scheduled session as scheduled', async () => {
    await expand(pastSession({ kind: 'scheduled', schedule_id: 'sched-1' }));
    expect(screen.getByText('Scheduled session')).toBeInTheDocument();
  });

  it('shows the YouTube study-mode allowlist', async () => {
    await expand(
      pastSession(
        {},
        {
          youtube_study_mode: true,
          youtube_blocked_channels: [],
          youtube_allowed_channels: ['@khanacademy'],
        },
      ),
    );
    expect(
      screen.getByText('Only these channels: @khanacademy'),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Blocked: @/)).not.toBeInTheDocument();
  });

  it('omits the YouTube row when nothing was configured for YouTube', async () => {
    await expand(
      pastSession(
        {},
        {
          youtube_blocked_channels: [],
          youtube_allowed_channels: [],
          youtube_study_mode: false,
        },
      ),
    );
    expect(screen.queryByText('YouTube')).not.toBeInTheDocument();
    // The rest of the detail still renders.
    expect(screen.getByText('One-shot session')).toBeInTheDocument();
    expect(screen.getByText('Blocked: instagram.com')).toBeInTheDocument();
  });
});
