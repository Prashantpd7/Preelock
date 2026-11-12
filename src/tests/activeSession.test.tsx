import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { fireEvent } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { ActiveSessionBanner } from '../components/ActiveSessionBanner';
import { AppLayout } from '../layouts/AppLayout';
import { useAppStore } from '../state/store';
import { api } from '../services/api';
import {
  browserEnforcement,
  effectiveWebsiteBlocking,
  elapsedMs,
  formatHMS,
  liveSession,
  liveTimeStats,
  scheduledEndMs,
  sessionTypeLabel,
  typeOfWorkLabel,
  youtubePolicy,
} from '../services/activeSession';
import {
  defaultFocus,
  type FocusConfig,
  type FocusSetup,
  type Session,
  type Snapshot,
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
    execute: vi.fn(),
    openSetup: vi.fn(),
    requestCamera: vi.fn(),
    blockAttempts: vi.fn(),
    appIcon: vi.fn(),
    exportStatistics: vi.fn(),
    deleteStatistics: vi.fn(),
    capabilities: vi.fn(),
    dismissBlockedAppPage: vi.fn(),
  },
}));

const baseSetup: FocusSetup = {
  platform: 'macos',
  native_available: true,
  applications: [{ id: 'com.example.focus-app', name: 'Focus App' }],
  camera_permission: 'granted',
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
  connected_browsers: [],
  browsers: [{ id: 'com.google.chrome', name: 'Chrome' }],
};

function connectedSetup(): FocusSetup {
  return {
    ...structuredClone(baseSetup),
    browser: {
      ...baseSetup.browser,
      connected: true,
      client_browser: 'com.google.chrome',
    },
    connected_browsers: ['com.google.chrome'],
  };
}

let seq = 0;
function makeSession(
  over: Partial<Session> = {},
  focusOver: Partial<FocusConfig> = {},
): Session {
  seq += 1;
  return {
    id: `s${seq}`,
    title: `Session ${seq}`,
    duration_minutes: 25,
    remaining_ms: 120_000,
    focused_ms: 0,
    allow_pause: true,
    status: 'running',
    started_at: Date.now() - 60_000,
    ended_at: null,
    deadline_ms: null,
    focus: {
      ...structuredClone(defaultFocus),
      ...focusOver,
    },
    ...over,
  };
}

function setData(data: Partial<Snapshot>) {
  useAppStore.setState({
    data: {
      sessions: [],
      tasks: [],
      schedules: [],
      one_shots: [],
      allowances: [],
      preferences: { theme: 'dark', default_minutes: 25 },
      ...data,
    },
    loading: false,
    error: null,
    busy: false,
  });
}

beforeEach(() => {
  seq = 0;
  vi.mocked(api.focusSetup).mockResolvedValue(structuredClone(baseSetup));
  vi.mocked(api.blockAttempts).mockResolvedValue([]);
  vi.mocked(api.capabilities).mockResolvedValue([]);
  vi.mocked(api.appIcon).mockResolvedValue(null);
  // AppLayout refreshes from the backend every second; answer
  // with the seeded store data so the seed survives the tick.
  vi.mocked(api.snapshot).mockImplementation(() =>
    Promise.resolve(useAppStore.getState().data),
  );
  setData({});
});

/** The banner contains router Links, so it renders inside a router. */
function renderBanner() {
  return render(
    <MemoryRouter>
      <ActiveSessionBanner />
    </MemoryRouter>,
  );
}

describe('activeSession helpers', () => {
  it('finds the live session and ignores finished ones', () => {
    const running = makeSession({ status: 'running' });
    const done = makeSession({ status: 'completed' });
    expect(liveSession([done, running])?.id).toBe(running.id);
    expect(liveSession([done])).toBeUndefined();
    expect(liveSession([])).toBeUndefined();
  });

  it('labels scheduled sessions and plain focus sessions', () => {
    expect(sessionTypeLabel(makeSession({ schedule_id: 'sched-1' }))).toBe(
      'Scheduled session',
    );
    expect(sessionTypeLabel(makeSession({}))).toBe('Focus session');
  });

  it('labels the recorded kind: one-shot, scheduled, and focus', () => {
    expect(sessionTypeLabel(makeSession({ kind: 'one_shot' }))).toBe(
      'One-shot session',
    );
    expect(sessionTypeLabel(makeSession({ kind: 'scheduled' }))).toBe(
      'Scheduled session',
    );
    expect(sessionTypeLabel(makeSession({ kind: 'focus' }))).toBe(
      'Focus session',
    );
    // The kind wins over a stale schedule id (a restarted schedule
    // session becomes a manual session).
    expect(
      sessionTypeLabel(makeSession({ kind: 'focus', schedule_id: 'sched-1' })),
    ).toBe('Focus session');
    // Sessions saved before the kind was recorded fall back to the
    // schedule id; an old one-shot has no marker and stays a focus
    // session.
    expect(sessionTypeLabel(makeSession({ schedule_id: 'sched-1' }))).toBe(
      'Scheduled session',
    );
  });

  it('reads live focus totals from the session and its attention buckets', () => {
    expect(
      liveTimeStats(
        makeSession({
          focused_ms: 300_000,
          attention: {
            state: 'FOCUSED',
            reason: '',
            elapsed_ms: 390_000,
            distracted_ms: 60_000,
            absent_ms: 30_000,
            unknown_ms: 0,
            corrected_ms: 0,
            intervals: [],
          },
        }),
      ),
    ).toEqual({
      focusedMs: 300_000,
      distractedMs: 60_000,
      awayMs: 30_000,
      totalMs: 390_000,
    });
    expect(liveTimeStats(makeSession({ focused_ms: 0 }))).toEqual({
      focusedMs: 0,
      distractedMs: 0,
      awayMs: 0,
      totalMs: 0,
    });
  });

  it('resolves website blocking: auto follows the mode', () => {
    expect(effectiveWebsiteBlocking({ ...defaultFocus, mode: 'strict' })).toBe(
      'allowlist',
    );
    expect(
      effectiveWebsiteBlocking({ ...defaultFocus, mode: 'standard' }),
    ).toBe('blocklist');
    expect(
      effectiveWebsiteBlocking({
        ...defaultFocus,
        website_blocking: 'none',
      }),
    ).toBe('none');
    expect(
      effectiveWebsiteBlocking({
        ...defaultFocus,
        website_blocking: 'allowlist',
      }),
    ).toBe('allowlist');
  });

  it('reads the YouTube policy: study, block, or none', () => {
    expect(
      youtubePolicy({
        ...defaultFocus,
        youtube_study_mode: true,
        youtube_allowed_channels: ['@khanacademy'],
        youtube_allow_watch_later: true,
      }),
    ).toEqual({
      mode: 'study',
      blocked: [],
      allowed: ['@khanacademy'],
      watchLater: true,
    });
    expect(
      youtubePolicy({
        ...defaultFocus,
        youtube_blocked_channels: ['@mkbhd'],
      }),
    ).toMatchObject({ mode: 'block', blocked: ['@mkbhd'] });
    expect(
      youtubePolicy({
        ...defaultFocus,
        youtube_block_entertainment: true,
      }),
    ).toEqual({
      mode: 'none',
      blocked: [],
      allowed: [],
      watchLater: false,
      entertainment: true,
    });
    expect(youtubePolicy(defaultFocus).mode).toBe('none');
  });

  it('reports browser enforcement only when the policy is applied', () => {
    const session = makeSession({ id: 'live-1' });
    const browser = {
      available: true,
      connected: true,
      applied_id: 'live-1',
      error: '',
      blocks: 0,
      cleanup_pending: false,
      extension_stale: false,
    };
    expect(
      browserEnforcement(
        {
          ...session,
          focus: {
            ...session.focus!,
            domains: ['youtube.com'],
          },
        },
        browser,
      ).state,
    ).toBe('enforced');
    expect(
      browserEnforcement(
        {
          ...session,
          focus: {
            ...session.focus!,
            domains: ['youtube.com'],
          },
        },
        { ...browser, applied_id: 'other' },
      ).state,
    ).toBe('connected-not-applied');
    expect(
      browserEnforcement(
        {
          ...session,
          focus: {
            ...session.focus!,
            domains: ['youtube.com'],
          },
        },
        { ...browser, connected: false, applied_id: '' },
      ).state,
    ).toBe('disconnected');
    // A session with no browser need never reports enforcement.
    expect(
      browserEnforcement(
        { ...session, focus: { ...session.focus!, domains: [] } },
        { ...browser, connected: false, applied_id: '' },
      ).state,
    ).toBe('not-needed');
  });

  it('formats HH:MM:SS and derives start/end/elapsed', () => {
    expect(formatHMS(0)).toBe('00:00:00');
    expect(formatHMS(3_661_000)).toBe('01:01:01');
    const session = makeSession({
      duration_minutes: 25,
      started_at: 1_000_000,
    });
    expect(scheduledEndMs(session)).toBe(1_000_000 + 25 * 60_000);
    expect(elapsedMs(session, 1_060_000)).toBe(60_000);
    expect(elapsedMs(makeSession({ started_at: null }), 1_060_000)).toBeNull();
  });

  it('labels the type of work', () => {
    expect(typeOfWorkLabel('coding')).toBe('Coding');
    expect(typeOfWorkLabel('lecture')).toBe('Watching a lesson');
    expect(typeOfWorkLabel('other')).toBe('General work');
  });
});

describe('ActiveSessionBanner', () => {
  it('shows nothing when no session is live', () => {
    setData({ sessions: [makeSession({ status: 'completed' })] });
    renderBanner();
    expect(
      screen.queryByRole('button', { name: /Active session/ }),
    ).not.toBeInTheDocument();
  });

  it('shows the live session with its timer and description', () => {
    setData({ sessions: [makeSession({ title: 'Deep work' })] });
    renderBanner();
    expect(
      screen.getByRole('button', { name: /Active session: Deep work/ }),
    ).toBeInTheDocument();
    expect(screen.getByText('02:00')).toBeInTheDocument();
    expect(
      screen.getByText(
        'Distracting apps and websites stay blocked until this session ends.',
      ),
    ).toBeInTheDocument();
  });

  it('updates the timer when the store refreshes', () => {
    const session = makeSession({ remaining_ms: 120_000 });
    setData({ sessions: [session] });
    renderBanner();
    expect(screen.getByText('02:00')).toBeInTheDocument();
    act(() => {
      useAppStore.setState((s) => ({
        data: {
          ...s.data,
          sessions: [{ ...session, remaining_ms: 60_000 }],
        },
      }));
    });
    expect(screen.getByText('01:00')).toBeInTheDocument();
    expect(screen.queryByText('02:00')).not.toBeInTheDocument();
  });

  it('reflects paused and preparing states', () => {
    setData({ sessions: [makeSession({ status: 'paused' })] });
    const { unmount } = renderBanner();
    expect(screen.getByText(/· paused/)).toBeInTheDocument();
    unmount();
    setData({ sessions: [makeSession({ status: 'preparing' })] });
    renderBanner();
    expect(screen.getByText(/· waiting for browser/)).toBeInTheDocument();
    expect(
      screen.getByText('Waiting for the browser extension to connect…'),
    ).toBeInTheDocument();
  });

  it('shows the grace countdown for an auto-started session', () => {
    setData({
      sessions: [makeSession({ grace_until_ms: Date.now() + 120_000 })],
    });
    renderBanner();
    expect(screen.getByText(/Starts in \d+s/)).toBeInTheDocument();
    expect(screen.getByText(/· starting soon/)).toBeInTheDocument();
  });

  it('disappears when the session completes', () => {
    const session = makeSession({});
    setData({ sessions: [session] });
    renderBanner();
    expect(
      screen.getByRole('button', { name: /Active session/ }),
    ).toBeInTheDocument();
    act(() => {
      useAppStore.setState((s) => ({
        data: {
          ...s.data,
          sessions: [{ ...session, status: 'completed' }],
        },
      }));
    });
    expect(
      screen.queryByRole('button', { name: /Active session/ }),
    ).not.toBeInTheDocument();
  });

  it('shows the reconnect hint when the browser is disconnected', () => {
    setData({
      sessions: [
        makeSession(
          {},
          { domains: ['youtube.com'], browser_app: 'com.google.chrome' },
        ),
      ],
      browser: { ...baseSetup.browser, connected: false },
    });
    renderBanner();
    expect(
      screen.getByText(
        'Browser disconnected — apps stay blocked. Reconnect to resume website blocking.',
      ),
    ).toBeInTheDocument();
  });
});

describe('ActiveSessionBanner pause/stop controls', () => {
  /** The store's execute applies the mocked api result, so make the
   *  mock apply pause/resume/end to the seeded store data. */
  function mockExecuteApplies() {
    vi.mocked(api.execute).mockImplementation(async (action) => {
      const data = useAppStore.getState().data;
      if (action.type === 'pause_session') {
        return {
          ...data,
          sessions: data.sessions.map((s) =>
            s.id === action.id ? { ...s, status: 'paused' } : s,
          ),
        };
      }
      if (action.type === 'resume_session') {
        return {
          ...data,
          sessions: data.sessions.map((s) =>
            s.id === action.id ? { ...s, status: 'running' } : s,
          ),
        };
      }
      if (action.type === 'end_session') {
        return {
          ...data,
          sessions: data.sessions.filter((s) => s.id !== action.id),
        };
      }
      return data;
    });
  }

  it('shows Pause and Stop for a flexible running session, and both work', async () => {
    mockExecuteApplies();
    const user = userEvent.setup();
    const session = makeSession({ title: 'Flex work' });
    setData({ sessions: [session] });
    renderBanner();
    expect(screen.getByRole('button', { name: 'Pause' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Stop' })).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Resume' }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Pause' }));
    expect(api.execute).toHaveBeenCalledWith({
      type: 'pause_session',
      id: session.id,
    });
    // Pausing shows the paused state on the banner (and never opens
    // the read-only details popup).
    expect(screen.getByText(/· paused/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Resume' })).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Pause' }),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Resume' }));
    expect(api.execute).toHaveBeenCalledWith({
      type: 'resume_session',
      id: session.id,
    });
    expect(screen.getByRole('button', { name: 'Pause' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Stop' }));
    expect(api.execute).toHaveBeenCalledWith({
      type: 'end_session',
      id: session.id,
    });
    // Stopping ends the session and the banner disappears.
    expect(
      screen.queryByRole('button', { name: /Active session/ }),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('shows no Pause or Stop for a strict running session', () => {
    setData({
      sessions: [makeSession({ title: 'Strict work' }, { mode: 'strict' })],
    });
    renderBanner();
    expect(
      screen.getByRole('button', { name: /Active session: Strict work/ }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Pause' }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Stop' }),
    ).not.toBeInTheDocument();
  });

  it('shows no Pause or Stop for a frozen session', () => {
    setData({
      sessions: [
        makeSession(
          { title: 'Frozen work' },
          {
            frozen: {
              enabled: true,
              allowed_apps: [],
              allowed_websites: [],
            },
          },
        ),
      ],
    });
    renderBanner();
    expect(
      screen.getByRole('button', { name: /Active session: Frozen work/ }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Pause' }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Stop' }),
    ).not.toBeInTheDocument();
  });

  it('shows Resume, not Pause, for a paused flexible session', () => {
    setData({
      sessions: [makeSession({ title: 'Paused work', status: 'paused' })],
    });
    renderBanner();
    expect(screen.getByRole('button', { name: 'Resume' })).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Pause' }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Stop' })).toBeInTheDocument();
  });

  it('shows no Pause when the session disallows pausing, but still offers Stop', () => {
    setData({
      sessions: [makeSession({ title: 'No-pause work', allow_pause: false })],
    });
    renderBanner();
    expect(
      screen.getByRole('button', { name: /Active session: No-pause work/ }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Pause' }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Stop' })).toBeInTheDocument();
  });
});

describe('ActiveSessionDetailsModal', () => {
  async function openModal(session: Session) {
    const user = userEvent.setup();
    setData({ sessions: [session] });
    renderBanner();
    await user.click(screen.getByRole('button', { name: /Active session/ }));
    const dialog = await screen.findByRole('dialog');
    return { user, dialog };
  }

  it('shows the overview: status, times and type — and nothing unselected', async () => {
    const session = makeSession(
      {
        title: 'Deep work',
        duration_minutes: 25,
        remaining_ms: 600_000,
        started_at: 1_700_000_000_000,
      },
      { attention: { ...defaultFocus.attention!, task: 'coding' } },
    );
    await openModal(session);
    expect(screen.getByText('Running')).toBeInTheDocument();
    expect(screen.getByText('00:10:00')).toBeInTheDocument();
    expect(screen.getByText('Focus session')).toBeInTheDocument();
    expect(screen.getByText('25 minutes')).toBeInTheDocument();
    // Type of work and the live attention state are not shown.
    expect(screen.queryByText('Type of work')).not.toBeInTheDocument();
    expect(screen.queryByText('Coding')).not.toBeInTheDocument();
    expect(screen.queryByText('Right now')).not.toBeInTheDocument();
  });

  it('shows a scheduled session as scheduled', async () => {
    await openModal(makeSession({ schedule_id: 'sched-1' }));
    expect(screen.getByText('Scheduled session')).toBeInTheDocument();
  });

  it('shows a one-shot session as a one-shot session', async () => {
    await openModal(makeSession({ kind: 'one_shot' }));
    expect(screen.getByText('One-shot session')).toBeInTheDocument();
  });

  it('shows a scheduled session by its recorded kind', async () => {
    await openModal(makeSession({ kind: 'scheduled' }));
    expect(screen.getByText('Scheduled session')).toBeInTheDocument();
  });

  it('shows live focus totals while the session runs', async () => {
    const session = makeSession({
      focused_ms: 120_000,
      attention: {
        state: 'FOCUSED',
        reason: '',
        elapsed_ms: 210_000,
        distracted_ms: 60_000,
        absent_ms: 30_000,
        unknown_ms: 0,
        corrected_ms: 0,
        intervals: [],
      },
    });
    await openModal(session);
    expect(screen.getByText('Time so far')).toBeInTheDocument();
    expect(screen.getByText('Focused 02:00')).toBeInTheDocument();
    expect(screen.getByText('Distracted 01:00')).toBeInTheDocument();
    expect(screen.getByText('Away 00:30')).toBeInTheDocument();
    expect(screen.getByText('Total 03:30')).toBeInTheDocument();
    // The popup re-reads the session from the store, so the totals
    // move as the session runs.
    setData({
      sessions: [
        {
          ...session,
          focused_ms: 300_000,
          attention: { ...session.attention!, distracted_ms: 120_000 },
        },
      ],
    });
    expect(await screen.findByText('Focused 05:00')).toBeInTheDocument();
    expect(screen.getByText('Distracted 02:00')).toBeInTheDocument();
    expect(screen.getByText('Total 07:30')).toBeInTheDocument();
  });

  it('omits the live totals when focus checks are off', async () => {
    await openModal(
      makeSession(
        {},
        { attention: { ...defaultFocus.attention!, enabled: false } },
      ),
    );
    expect(screen.queryByText('Time so far')).not.toBeInTheDocument();
    expect(screen.queryByText('Focused')).not.toBeInTheDocument();
  });

  it('lists allowed work apps and blocked folders', async () => {
    vi.mocked(api.focusSetup).mockResolvedValue(connectedSetup());
    await openModal(
      makeSession(
        {},
        {
          apps: ['com.example.focus-app'],
          blocked_folders: ['/Applications/Games'],
          blocked_titles: ['YouTube'],
        },
      ),
    );
    expect(await screen.findByText('Focus App')).toBeInTheDocument();
    expect(screen.getByText('Allowed')).toBeInTheDocument();
    expect(screen.getByText('/Applications/Games')).toBeInTheDocument();
    expect(screen.getByText('YouTube')).toBeInTheDocument();
  });

  it('shows blocklist websites and exceptions', async () => {
    await openModal(
      makeSession(
        {},
        {
          domains: ['youtube.com', 'instagram.com'],
          exceptions: ['github.com'],
        },
      ),
    );
    expect(screen.getByText('Block distracting websites')).toBeInTheDocument();
    expect(screen.getByText('youtube.com')).toBeInTheDocument();
    expect(screen.getByText('instagram.com')).toBeInTheDocument();
    expect(screen.getByText('github.com')).toBeInTheDocument();
  });

  it('shows allowlist websites and the everything-else-blocked note', async () => {
    await openModal(
      makeSession(
        {},
        { website_blocking: 'allowlist', domains: ['github.com'] },
      ),
    );
    expect(
      screen.getByText('Block all websites except these'),
    ).toBeInTheDocument();
    expect(screen.getByText('github.com')).toBeInTheDocument();
    expect(
      screen.getByText('Every other website is blocked.'),
    ).toBeInTheDocument();
  });

  it('omits the website section when website blocking is off', async () => {
    await openModal(makeSession({}, { website_blocking: 'none', domains: [] }));
    expect(screen.queryByText('Website restrictions')).not.toBeInTheDocument();
    expect(
      screen.queryByText('Website blocking is off for this session.'),
    ).not.toBeInTheDocument();
  });

  it('shows YouTube channel blocklist and Watch Later', async () => {
    await openModal(
      makeSession(
        {},
        {
          youtube_blocked_channels: ['@mkbhd'],
          youtube_allow_watch_later: true,
        },
      ),
    );
    expect(
      screen.getByText('These channels and their videos are blocked.'),
    ).toBeInTheDocument();
    expect(screen.getByText('@mkbhd')).toBeInTheDocument();
    expect(
      screen.getByText('Watch Later exception is enabled.'),
    ).toBeInTheDocument();
  });

  it('shows YouTube study mode allowlist', async () => {
    await openModal(
      makeSession(
        {},
        {
          youtube_study_mode: true,
          youtube_allowed_channels: ['@khanacademy'],
          youtube_allow_watch_later: false,
        },
      ),
    );
    expect(
      screen.getByText('Only the listed channels are allowed.'),
    ).toBeInTheDocument();
    expect(screen.getByText('@khanacademy')).toBeInTheDocument();
    // Watch Later is off, so no Watch Later line appears at all.
    expect(
      screen.queryByText('Watch Later exception is enabled.'),
    ).not.toBeInTheDocument();
  });

  it('omits the YouTube section when nothing is configured', async () => {
    await openModal(makeSession({}, {}));
    expect(screen.queryByText('YouTube restrictions')).not.toBeInTheDocument();
    expect(
      screen.queryByText('No YouTube channel restrictions configured.'),
    ).not.toBeInTheDocument();
  });

  it('never shows the browser extension section, even when disconnected', async () => {
    await openModal(
      makeSession(
        {},
        { domains: ['youtube.com'], browser_app: 'com.google.chrome' },
      ),
    );
    expect(screen.queryByText('Browser extension')).not.toBeInTheDocument();
    expect(screen.queryByText('Not connected')).not.toBeInTheDocument();
    expect(screen.queryByText('Not enforced')).not.toBeInTheDocument();
  });

  it('never shows the browser extension section, even when enforced', async () => {
    vi.mocked(api.focusSetup).mockResolvedValue(connectedSetup());
    const session = makeSession(
      { id: 'live-1' },
      { domains: ['youtube.com'], browser_app: 'com.google.chrome' },
    );
    const user = userEvent.setup();
    setData({
      sessions: [session],
      browser: {
        ...baseSetup.browser,
        connected: true,
        applied_id: 'live-1',
        client_browser: 'com.google.chrome',
      },
    });
    renderBanner();
    await user.click(screen.getByRole('button', { name: /Active session/ }));
    await screen.findByRole('dialog');
    expect(screen.queryByText('Browser extension')).not.toBeInTheDocument();
    expect(screen.queryByText('Connected')).not.toBeInTheDocument();
    expect(screen.queryByText('Enforced')).not.toBeInTheDocument();
    expect(
      screen.queryByText(
        'The extension has applied this session’s website rules in the browser.',
      ),
    ).not.toBeInTheDocument();
  });

  it('shows only what is configured: no optional sections for a bare session', async () => {
    await openModal(
      makeSession(
        {},
        {
          website_blocking: 'none',
          activity_consent: false,
          attention: { ...defaultFocus.attention!, enabled: false },
        },
      ),
    );
    // Overview and apps are always there; everything optional is gone.
    expect(screen.getByText('Session overview')).toBeInTheDocument();
    expect(screen.getByText('Selected apps')).toBeInTheDocument();
    expect(screen.queryByText('Website restrictions')).not.toBeInTheDocument();
    expect(screen.queryByText('YouTube restrictions')).not.toBeInTheDocument();
    expect(screen.queryByText('Other restrictions')).not.toBeInTheDocument();
    expect(screen.queryByText('Browser extension')).not.toBeInTheDocument();
    for (const label of [
      'Focus checks',
      'Camera',
      'Pomodoro',
      'Reward breaks',
      'Frozen',
      'Locks',
    ]) {
      expect(screen.queryByText(label)).not.toBeInTheDocument();
    }
    expect(screen.queryByText('Block other apps: On')).not.toBeInTheDocument();
  });

  it('shows Other restrictions rows only for enabled features', async () => {
    await openModal(
      makeSession(
        { lock: { password_hash: 'deadbeef' } },
        {
          camera: true,
          pomodoro: { ...defaultFocus.pomodoro!, enabled: true },
          reward: { ...defaultFocus.reward!, enabled: true },
          frozen: { ...defaultFocus.frozen!, enabled: true },
        },
      ),
    );
    expect(screen.getByText('Other restrictions')).toBeInTheDocument();
    expect(screen.getByText('Focus checks')).toBeInTheDocument();
    expect(screen.getByText('Camera')).toBeInTheDocument();
    expect(screen.getByText('Pomodoro')).toBeInTheDocument();
    expect(screen.getByText('Reward breaks')).toBeInTheDocument();
    expect(screen.getByText('Frozen')).toBeInTheDocument();
    expect(screen.getByText('Locks')).toBeInTheDocument();
    expect(screen.getByText('password')).toBeInTheDocument();
  });

  it('closes with the X button and leaves the session untouched', async () => {
    const session = makeSession({ status: 'running', remaining_ms: 90_000 });
    const { user } = await openModal(session);
    const before = useAppStore.getState().data.sessions[0];
    await user.click(screen.getByRole('button', { name: 'Close dialog' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    const after = useAppStore.getState().data.sessions[0];
    expect(after).toEqual(before);
    expect(after.status).toBe('running');
    expect(after.remaining_ms).toBe(90_000);
    // The banner is still there: closing the popup changed nothing.
    expect(
      screen.getByRole('button', { name: /Active session/ }),
    ).toBeInTheDocument();
  });

  it('closes with Escape and leaves the session untouched', async () => {
    const session = makeSession({ status: 'running' });
    await openModal(session);
    const dialog = screen.getByRole('dialog');
    // jsdom does not turn an Escape keypress into a dialog `cancel`
    // event, so dispatch the event a real browser fires on Escape.
    // The Modal wires it to onClose, exactly like every other modal.
    fireEvent(dialog, new Event('cancel', { bubbles: false }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(useAppStore.getState().data.sessions[0].status).toBe('running');
  });

  it('updates live when the session is paused', async () => {
    const session = makeSession({ status: 'running' });
    await openModal(session);
    expect(screen.getByText('Running')).toBeInTheDocument();
    act(() => {
      useAppStore.setState((s) => ({
        data: {
          ...s.data,
          sessions: [{ ...session, status: 'paused' }],
        },
      }));
    });
    expect(await screen.findByText('Paused')).toBeInTheDocument();
  });
});

describe('AppLayout global banner', () => {
  it('shows the banner on Analytics, not just the Dashboard', () => {
    setData({ sessions: [makeSession({ title: 'Deep work' })] });
    const { unmount } = render(
      <MemoryRouter initialEntries={['/analytics']}>
        <AppLayout />
      </MemoryRouter>,
    );
    expect(
      screen.getByRole('button', { name: /Active session: Deep work/ }),
    ).toBeInTheDocument();
    unmount();
  });

  it('shows the banner on Settings too', () => {
    setData({ sessions: [makeSession({ title: 'Deep work' })] });
    const { unmount } = render(
      <MemoryRouter initialEntries={['/settings']}>
        <AppLayout />
      </MemoryRouter>,
    );
    expect(
      screen.getByRole('button', { name: /Active session: Deep work/ }),
    ).toBeInTheDocument();
    unmount();
  });

  it('shows no banner anywhere when nothing is live', () => {
    setData({ sessions: [] });
    const { unmount } = render(
      <MemoryRouter initialEntries={['/tasks']}>
        <AppLayout />
      </MemoryRouter>,
    );
    expect(
      screen.queryByRole('button', { name: /Active session/ }),
    ).not.toBeInTheDocument();
    unmount();
  });
});
