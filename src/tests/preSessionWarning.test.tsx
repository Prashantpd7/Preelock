import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { PreSessionWarning } from '../components/PreSessionWarning';
import { missingPrerequisites } from '../services/focusSetup';
import {
  autoStartedLiveSessions,
  minutesUntilStart,
  upcomingStarts,
} from '../services/preSessionWarning';
import { nextScheduleStartMs } from '../services/schedules';
import { useAppStore } from '../state/store';
import { api } from '../services/api';
import {
  defaultFocus,
  type FocusConfig,
  type FocusReport,
  type FocusSetup,
  type OneShot,
  type Schedule,
  type Session,
  type Snapshot,
} from '../types';

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

const baseSetup: FocusSetup = {
  platform: 'macos',
  native_available: true,
  applications: [],
  camera_permission: 'granted',
  pairing_token: 'ABC123',
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

function disconnectedSetup(): FocusSetup {
  return {
    ...structuredClone(baseSetup),
    camera_permission: 'granted',
    browser: { ...baseSetup.browser, connected: false },
    connected_browsers: [],
  };
}

function connectedSetup(): FocusSetup {
  return {
    ...structuredClone(baseSetup),
    camera_permission: 'granted',
    browser: {
      ...baseSetup.browser,
      connected: true,
      client_browser: 'com.google.chrome',
    },
    connected_browsers: ['com.google.chrome'],
  };
}

function cameraMissingSetup(): FocusSetup {
  return {
    ...structuredClone(baseSetup),
    camera_permission: 'not_requested',
    browser: {
      ...baseSetup.browser,
      connected: true,
      client_browser: 'com.google.chrome',
    },
    connected_browsers: ['com.google.chrome'],
  };
}

function browserFocus(): FocusConfig {
  return {
    ...structuredClone(defaultFocus),
    domains: ['example.com'],
    browser_app: 'com.google.chrome',
  };
}

function cameraFocus(): FocusConfig {
  return {
    ...structuredClone(defaultFocus),
    camera: true,
    attention: {
      ...structuredClone(defaultFocus.attention!),
      verification: 'camera_and_activity',
    },
  };
}

function hhmm(date: Date): string {
  return `${String(date.getHours()).padStart(2, '0')}:${String(
    date.getMinutes(),
  ).padStart(2, '0')}`;
}

let seq = 0;
function makeSchedule(over: Partial<Schedule> = {}): Schedule {
  seq += 1;
  const now = new Date();
  const start = new Date(now.getTime() + 4 * 60_000);
  const end = new Date(now.getTime() + 64 * 60_000);
  return {
    id: `sched${seq}`,
    title: `Schedule ${seq}`,
    days: [now.getDay()],
    start_time: hhmm(start),
    end_time: hhmm(end),
    focus: structuredClone(defaultFocus),
    enabled: true,
    lock_changes: false,
    created_at: Date.now(),
    ...over,
  };
}

function makeOneShot(over: Partial<OneShot> = {}): OneShot {
  seq += 1;
  return {
    id: `one${seq}`,
    title: `One-shot ${seq}`,
    minutes: 25,
    allow_pause: true,
    focus: structuredClone(defaultFocus),
    fire_at_ms: Date.now() + 3 * 60_000,
    created_at: Date.now(),
    ...over,
  };
}

// A report with no events: Session.report is optional, so spreading
// `makeSession().report` would widen every field to `| undefined`.
const baseReport: FocusReport = {
  events: [],
  approved_ms: 0,
  distracting_ms: 0,
  idle_ms: 0,
  interruptions: 0,
  active_app: '',
  enforcement: '',
  camera_status: '',
  browser_blocks: 0,
};

function makeSession(over: Partial<Session> = {}): Session {
  seq += 1;
  return {
    id: `sess${seq}`,
    title: `Session ${seq}`,
    duration_minutes: 25,
    remaining_ms: 25 * 60_000,
    focused_ms: 0,
    allow_pause: true,
    status: 'running',
    started_at: Date.now(),
    ended_at: null,
    deadline_ms: null,
    focus: structuredClone(defaultFocus),
    report: structuredClone(baseReport),
    ...over,
  };
}

function setData(over: Partial<Snapshot> = {}) {
  useAppStore.setState({
    data: {
      sessions: [],
      tasks: [],
      schedules: [],
      one_shots: [],
      allowances: [],
      preferences: { theme: 'dark', default_minutes: 25 },
      ...over,
    } as Snapshot,
    loading: false,
    error: null,
    busy: false,
  });
}

beforeEach(() => {
  seq = 0;
  vi.clearAllMocks();
  vi.useFakeTimers({
    toFake: ['Date'],
    now: new Date(2026, 4, 6, 12, 0, 0),
  });
  localStorage.clear();
  HTMLDialogElement.prototype.showModal = function () {
    this.setAttribute('open', '');
  };
  HTMLDialogElement.prototype.close = function () {
    this.removeAttribute('open');
  };
  setData();
  vi.mocked(api.focusSetup).mockResolvedValue(disconnectedSetup());
  vi.mocked(api.requestCamera).mockResolvedValue(undefined);
  vi.mocked(api.openSetup).mockResolvedValue(undefined);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('nextScheduleStartMs', () => {
  it('finds today when the start is still ahead', () => {
    const now = new Date();
    const start = new Date(now.getTime() + 30 * 60_000);
    const schedule = makeSchedule({
      days: [now.getDay()],
      start_time: hhmm(start),
    });
    const next = nextScheduleStartMs(schedule, now.getTime());
    expect(next).not.toBeNull();
    expect(next! - now.getTime()).toBeGreaterThan(0);
    expect(next! - now.getTime()).toBeLessThanOrEqual(30 * 60_000);
  });

  it('moves to the next listed day when today already passed', () => {
    const now = new Date();
    const passed = new Date(now.getTime() - 30 * 60_000);
    const tomorrow = (now.getDay() + 1) % 7;
    const schedule = makeSchedule({
      days: [now.getDay(), tomorrow],
      start_time: hhmm(passed),
    });
    const next = nextScheduleStartMs(schedule, now.getTime());
    expect(next).not.toBeNull();
    expect(new Date(next!).getDay()).toBe(tomorrow);
  });

  it('wraps to next week when only today is listed and the start passed', () => {
    const now = new Date();
    const passed = new Date(now.getTime() - 30 * 60_000);
    const schedule = makeSchedule({
      days: [now.getDay()],
      start_time: hhmm(passed),
    });
    const next = nextScheduleStartMs(schedule, now.getTime());
    expect(next).not.toBeNull();
    expect(next! - now.getTime()).toBeGreaterThan(6 * 86_400_000);
  });

  it('returns null with no listed days or a bad start time', () => {
    expect(
      nextScheduleStartMs(makeSchedule({ days: [] }), Date.now()),
    ).toBeNull();
    expect(
      nextScheduleStartMs(makeSchedule({ start_time: '99:99' }), Date.now()),
    ).toBeNull();
  });
});

describe('missingPrerequisites', () => {
  it('reports nothing for a plain session', () => {
    expect(missingPrerequisites(defaultFocus, disconnectedSetup())).toEqual({
      browser: false,
      camera: false,
    });
  });

  it('reports the browser only when website blocking needs one and it is not connected', () => {
    expect(missingPrerequisites(browserFocus(), disconnectedSetup())).toEqual({
      browser: true,
      camera: false,
    });
    expect(missingPrerequisites(browserFocus(), connectedSetup())).toEqual({
      browser: false,
      camera: false,
    });
  });

  it('reports the browser when the connected extension lives in another browser', () => {
    const wrong = connectedSetup();
    wrong.browser.client_browser = 'com.brave.browser';
    wrong.connected_browsers = ['com.brave.browser'];
    expect(missingPrerequisites(browserFocus(), wrong)).toEqual({
      browser: true,
      camera: false,
    });
  });

  it('reports the camera only when the session needs it and access is not granted', () => {
    expect(missingPrerequisites(cameraFocus(), cameraMissingSetup())).toEqual({
      browser: false,
      camera: true,
    });
    expect(missingPrerequisites(cameraFocus(), connectedSetup())).toEqual({
      browser: false,
      camera: false,
    });
    expect(missingPrerequisites(defaultFocus, cameraMissingSetup())).toEqual({
      browser: false,
      camera: false,
    });
  });

  it('never reports the camera on Windows', () => {
    const windows = { ...cameraMissingSetup(), platform: 'windows' };
    expect(missingPrerequisites(cameraFocus(), windows)).toEqual({
      browser: false,
      camera: false,
    });
  });

  it('reports nothing without setup data', () => {
    expect(missingPrerequisites(browserFocus(), null)).toEqual({
      browser: false,
      camera: false,
    });
  });

  it('reports both when both are missing', () => {
    const both = { ...browserFocus(), camera: true };
    const setup = {
      ...disconnectedSetup(),
      camera_permission: 'denied',
    };
    expect(missingPrerequisites(both, setup)).toEqual({
      browser: true,
      camera: true,
    });
  });
});

describe('upcomingStarts', () => {
  it('finds a schedule and a one-shot inside the window, soonest first', () => {
    const now = Date.now();
    const data = {
      sessions: [],
      schedules: [makeSchedule({ title: 'Later' })],
      one_shots: [makeOneShot({ title: 'Sooner', fire_at_ms: now + 60_000 })],
    } as unknown as Snapshot;
    const found = upcomingStarts(data, now);
    expect(found.map((s) => s.title)).toEqual(['Sooner', 'Later']);
  });

  it('ignores starts outside the window, disabled schedules, and already-live schedule sessions', () => {
    const now = Date.now();
    const live = makeSession({
      schedule_id: 'sched-live',
      status: 'running',
    });
    const data = {
      sessions: [live],
      schedules: [
        makeSchedule({ id: 'sched-far', title: 'Far' }),
        makeSchedule({ id: 'sched-live', title: 'Live' }),
        makeSchedule({ id: 'sched-off', title: 'Off', enabled: false }),
      ],
      one_shots: [
        makeOneShot({ title: 'Too far', fire_at_ms: now + 30 * 60_000 }),
      ],
    } as unknown as Snapshot;
    // The 'Far' schedule starts in ~4 minutes: it is the only one due.
    expect(upcomingStarts(data, now).map((s) => s.title)).toEqual(['Far']);
  });
});

describe('autoStartedLiveSessions', () => {
  it('matches schedule sessions and one-shot sessions, not manual ones', () => {
    const scheduled = makeSession({
      schedule_id: 'sched1',
      report: {
        ...baseReport,
        events: [
          {
            at: Date.now(),
            kind: 'schedule_started',
            detail: 'Scheduled block started automatically.',
          },
        ],
      },
    });
    const oneShot = makeSession({
      report: {
        ...baseReport,
        events: [
          {
            at: Date.now(),
            kind: 'one_shot_started',
            detail: 'One-shot block started automatically.',
          },
        ],
      },
    });
    const manual = makeSession();
    const done = makeSession({ status: 'completed', schedule_id: 'sched1' });
    const data = {
      sessions: [scheduled, oneShot, manual, done],
    } as unknown as Snapshot;
    expect(autoStartedLiveSessions(data).map((s) => s.id)).toEqual([
      scheduled.id,
      oneShot.id,
    ]);
  });
});

describe('minutesUntilStart', () => {
  it('rounds up, never below one', () => {
    expect(minutesUntilStart(10_000_000, 10_000_000 - 30_000)).toBe(1);
    expect(minutesUntilStart(10_000_000, 10_000_000 - 5 * 60_000)).toBe(5);
  });
});

describe('PreSessionWarning popup', () => {
  it('warns a schedule starting in 5 minutes with a disconnected browser', async () => {
    const schedule = makeSchedule({ focus: browserFocus() });
    setData({ schedules: [schedule] });
    render(<PreSessionWarning />);
    // The title flips to the browser variant once the per-session
    // setup check lands, so wait for the text, not just the dialog.
    // findByRole (not a sync getByRole) because the native <dialog>
    // only exposes the dialog role once Modal's showModal effect
    // has run — under load the text can land first.
    await screen.findByText('Connect browser to start this session');
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent(schedule.title);
    expect(dialog).toHaveTextContent(/starts in \d+ minutes/);
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Connect browser' }));
    expect(await screen.findByText(/Browser is not connected\./)).toBeVisible();
  });

  it('warns a one-shot starting in 5 minutes with camera access missing', async () => {
    vi.mocked(api.focusSetup).mockResolvedValue(cameraMissingSetup());
    const shot = makeOneShot({ focus: cameraFocus() });
    setData({ one_shots: [shot] });
    render(<PreSessionWarning />);
    await screen.findByText('Allow camera to start this session');
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Allow camera' }));
    expect(api.requestCamera).toHaveBeenCalledTimes(1);
    expect(await screen.findByText(/Answer the camera prompt/)).toBeVisible();
  });

  it('shows a plain notice when everything is ready', async () => {
    vi.mocked(api.focusSetup).mockResolvedValue(connectedSetup());
    const schedule = makeSchedule({ focus: browserFocus() });
    setData({ schedules: [schedule] });
    render(<PreSessionWarning />);
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent('Session starting soon');
    expect(
      screen.queryByRole('button', { name: 'Connect browser' }),
    ).toBeNull();
    expect(screen.queryByRole('button', { name: 'Allow camera' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Got it' })).toBeVisible();
  });

  it('stays silent when the start is more than 5 minutes away', async () => {
    const schedule = makeSchedule({ title: 'Later' });
    // Push the start out: tomorrow, same time.
    schedule.days = [(new Date().getDay() + 1) % 7];
    setData({ schedules: [schedule] });
    render(<PreSessionWarning />);
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('dismissing the popup does not cancel the schedule or one-shot', async () => {
    const schedule = makeSchedule({ focus: browserFocus() });
    const shot = makeOneShot({ focus: browserFocus() });
    setData({ schedules: [schedule], one_shots: [shot] });
    render(<PreSessionWarning />);
    await screen.findByRole('dialog');
    // Dismiss via the dialog's close button: closing never touches
    // the schedule or the one-shot.
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Close dialog' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    // The schedule and the one-shot are untouched: nothing was
    // cancelled, ended, or deleted, and no session was started.
    const state = useAppStore.getState().data;
    expect(state.schedules).toHaveLength(1);
    expect(state.one_shots).toHaveLength(1);
    expect(state.sessions).toHaveLength(0);
    // A rerender does not bring the dismissed popup back.
    setData({ schedules: [schedule], one_shots: [shot] });
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('shows the popup again at start time when the browser is still missing', async () => {
    const session = makeSession({
      title: 'Morning block',
      schedule_id: 'sched1',
      status: 'running',
      started_at: Date.now(),
      focus: browserFocus(),
      report: {
        ...baseReport,
        events: [
          {
            at: Date.now(),
            kind: 'schedule_started',
            detail: 'Scheduled block started automatically.',
          },
        ],
      },
    });
    setData({ sessions: [session] });
    render(<PreSessionWarning />);
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent(
      /started, but the browser is not connected/,
    );
    await screen.findByText('Connect browser to start this session');
  });

  it('does not show the at-start popup for a manual live session', async () => {
    const session = makeSession({ focus: browserFocus() });
    setData({ sessions: [session] });
    render(<PreSessionWarning />);
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('retries live warning check after transient focusSetup failure and closes when the live session ends', async () => {
    vi.mocked(api.focusSetup)
      .mockRejectedValueOnce(new Error('transient'))
      .mockResolvedValue(disconnectedSetup());
    const live = makeSession({
      id: 'live-retry-1',
      title: 'Morning block',
      schedule_id: 'sched1',
      status: 'running',
      started_at: Date.now(),
      focus: browserFocus(),
    });
    setData({ sessions: [live] });
    render(<PreSessionWarning />);
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(screen.queryByRole('dialog')).toBeNull();

    // Subsequent snapshot poll retries because transient failure rolled back shownLive.
    setData({ sessions: [{ ...live, remaining_ms: 59 * 60_000 }] });
    expect(await screen.findByRole('dialog')).toHaveTextContent(
      'Morning block',
    );

    // Completing the session auto-closes the stale live warning.
    setData({ sessions: [{ ...live, status: 'completed', remaining_ms: 0 }] });
    await vi.waitFor(() => {
      expect(screen.queryByRole('dialog')).toBeNull();
    });
  });
});
