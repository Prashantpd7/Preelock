import { describe, expect, it } from 'vitest';
import {
  analyticsSummary,
  focusedMsByDay,
  formatShortDuration,
  periodDays,
} from '../services/analytics';
import { defaultFocus, type Session } from '../types';

const NOW = new Date(2026, 9, 19, 12); // Oct 19 2026, noon local time
const at = (day: number, hour: number, minute = 0) =>
  new Date(2026, 9, day, hour, minute).getTime();

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
const legacyFocus = { ...defaultFocus, attention: undefined };
function interval(
  startMs: number,
  durationMs: number,
  state: string,
  correctedMs = 0,
) {
  return {
    id: `i${startMs}`,
    at: startMs,
    duration_ms: durationMs,
    state,
    reason: '',
    corrected_ms: correctedMs,
    correction_note: '',
  };
}
function attentionWith(intervals: ReturnType<typeof interval>[]) {
  return {
    state: 'UNKNOWN',
    reason: '',
    elapsed_ms: 0,
    distracted_ms: 0,
    absent_ms: 0,
    unknown_ms: 0,
    corrected_ms: 0,
    intervals,
  };
}

describe('formatShortDuration', () => {
  it('never shows a positive duration as 0m', () => {
    expect(formatShortDuration(0)).toBe('0m');
    expect(formatShortDuration(-5)).toBe('0m');
    expect(formatShortDuration(500)).toBe('<1s');
    expect(formatShortDuration(999)).toBe('<1s');
    expect(formatShortDuration(1000)).toBe('1s');
    expect(formatShortDuration(45_000)).toBe('45s');
    expect(formatShortDuration(59_999)).toBe('59s');
  });
  it('formats minutes and hours compactly', () => {
    expect(formatShortDuration(60_000)).toBe('1m');
    expect(formatShortDuration(28 * 60_000)).toBe('28m');
    expect(formatShortDuration(52 * 60_000)).toBe('52m');
    expect(formatShortDuration(3_600_000)).toBe('1h');
    expect(formatShortDuration(90 * 60_000)).toBe('1h 30m');
    expect(formatShortDuration(275 * 60_000)).toBe('4h 35m');
  });
});

describe('sessions completed counts', () => {
  it('shows 2/6 for 2 completed + 4 ended/interrupted sessions', () => {
    const sessions = [
      makeSession({
        status: 'completed',
        started_at: at(19, 9),
        focused_ms: 60_000,
      }),
      makeSession({
        status: 'completed',
        started_at: at(19, 10),
        focused_ms: 60_000,
      }),
      makeSession({
        status: 'ended',
        started_at: at(19, 11),
        focused_ms: 30_000,
      }),
      makeSession({ status: 'ended', started_at: at(18, 9), focused_ms: 0 }),
      makeSession({
        status: 'interrupted',
        started_at: at(18, 10),
        focused_ms: 10_000,
      }),
      makeSession({
        status: 'interrupted',
        started_at: at(17, 9),
        focused_ms: 0,
      }),
    ];
    const summary = analyticsSummary(sessions, 'week', NOW);
    expect(summary.completedCount).toBe(2);
    expect(summary.finishedCount).toBe(6);
  });
  it('excludes planned, preparing, running and paused sessions', () => {
    const sessions = [
      makeSession({
        status: 'completed',
        started_at: at(19, 9),
        focused_ms: 60_000,
      }),
      makeSession({ status: 'ready', started_at: null }),
      makeSession({
        status: 'preparing',
        started_at: at(19, 9),
        focused_ms: 5_000,
      }),
      makeSession({
        status: 'running',
        started_at: at(19, 9),
        focused_ms: 5_000,
      }),
      makeSession({
        status: 'paused',
        started_at: at(19, 9),
        focused_ms: 5_000,
      }),
    ];
    const summary = analyticsSummary(sessions, 'week', NOW);
    expect(summary.completedCount).toBe(1);
    expect(summary.finishedCount).toBe(1);
  });
  it('excludes legacy elapsed-time sessions from every card', () => {
    const sessions = [
      makeSession({
        status: 'completed',
        started_at: at(19, 9),
        focused_ms: 60_000,
        focus: legacyFocus,
      }),
    ];
    const summary = analyticsSummary(sessions, 'week', NOW);
    expect(summary.hasEligible).toBe(false);
    expect(summary.focusMs).toBe(0);
    expect(summary.finishedCount).toBe(0);
  });
});

describe('date filters and local calendar boundaries', () => {
  const sessions = [
    makeSession({
      status: 'completed',
      started_at: at(19, 9),
      focused_ms: 60_000,
    }),
    makeSession({
      status: 'completed',
      started_at: at(18, 9),
      focused_ms: 60_000,
    }),
    makeSession({ status: 'ended', started_at: at(13, 9), focused_ms: 60_000 }),
    makeSession({ status: 'ended', started_at: at(12, 9), focused_ms: 60_000 }),
    makeSession({
      status: 'completed',
      started_at: at(1, 9),
      focused_ms: 60_000,
    }),
  ];
  it('filters finished sessions by start date', () => {
    expect(analyticsSummary(sessions, 'today', NOW).finishedCount).toBe(1);
    expect(analyticsSummary(sessions, 'week', NOW).finishedCount).toBe(3);
    expect(analyticsSummary(sessions, 'month', NOW).finishedCount).toBe(5);
  });
  it('counts calendar days per period', () => {
    expect(analyticsSummary(sessions, 'today', NOW).dayCount).toBe(1);
    expect(analyticsSummary(sessions, 'week', NOW).dayCount).toBe(7);
    expect(analyticsSummary(sessions, 'month', NOW).dayCount).toBe(19);
  });
  it('counts all-time days from the first tracked session', () => {
    const summary = analyticsSummary(sessions, 'all', NOW);
    expect(summary.dayCount).toBe(19);
    expect(summary.finishedCount).toBe(5);
    expect(periodDays('all', NOW, sessions).length).toBe(19);
  });
});

describe('midnight-crossing attribution', () => {
  it('splits FOCUSED intervals at local midnight', () => {
    const sessions = [
      makeSession({
        status: 'completed',
        started_at: at(18, 23, 30),
        ended_at: at(19, 0, 30),
        focused_ms: 3_600_000,
        attention: attentionWith([
          interval(at(18, 23, 30), 3_600_000, 'FOCUSED'),
        ]),
      }),
    ];
    const byDay = focusedMsByDay(sessions);
    expect(byDay.get('2026-10-18')).toBe(1_800_000);
    expect(byDay.get('2026-10-19')).toBe(1_800_000);
    const today = analyticsSummary(sessions, 'today', NOW);
    expect(today.focusMs).toBe(1_800_000);
    expect(today.days[0].ms).toBe(1_800_000);
  });
  it('falls back to the start date without interval history', () => {
    const sessions = [
      makeSession({
        status: 'completed',
        started_at: at(18, 10),
        focused_ms: 600_000,
      }),
    ];
    const byDay = focusedMsByDay(sessions);
    expect(byDay.get('2026-10-18')).toBe(600_000);
    expect(byDay.get('2026-10-19')).toBeUndefined();
  });
  it('attributes corrected time to the interval date', () => {
    const sessions = [
      makeSession({
        status: 'completed',
        started_at: at(19, 9),
        focused_ms: 30_000,
        attention: attentionWith([
          interval(at(19, 9), 60_000, 'UNKNOWN', 30_000),
        ]),
      }),
    ];
    expect(focusedMsByDay(sessions).get('2026-10-19')).toBe(30_000);
  });
});

describe('days focused and streaks', () => {
  const sessions = [19, 18, 17, 15, 14].map((day) =>
    makeSession({
      status: 'completed',
      started_at: at(day, 9),
      focused_ms: 60_000,
    }),
  );
  it('counts days with focus time and the longest consecutive run', () => {
    const summary = analyticsSummary(sessions, 'week', NOW);
    expect(summary.daysFocused).toBe(5);
    expect(summary.dayCount).toBe(7);
    expect(summary.longestStreakDays).toBe(3);
  });
  it('reports 1/1 for a focused today and 0/1 without focus', () => {
    const withFocus = analyticsSummary(sessions, 'today', NOW);
    expect(withFocus.daysFocused).toBe(1);
    const withoutFocus = analyticsSummary([], 'today', NOW);
    expect(withoutFocus.daysFocused).toBe(0);
    expect(withoutFocus.dayCount).toBe(1);
    expect(withoutFocus.longestStreakDays).toBe(0);
  });
});

describe('average and best session', () => {
  it('averages finished sessions including zero-focus ones', () => {
    const sessions = [
      makeSession({
        title: 'Alpha',
        status: 'completed',
        started_at: at(19, 9),
        focused_ms: 600_000,
      }),
      makeSession({
        title: 'Beta',
        status: 'ended',
        started_at: at(19, 10),
        focused_ms: 0,
      }),
      makeSession({
        title: 'Gamma',
        status: 'interrupted',
        started_at: at(18, 9),
        focused_ms: 1_200_000,
      }),
    ];
    const summary = analyticsSummary(sessions, 'week', NOW);
    expect(summary.averageMs).toBe(600_000);
    expect(summary.best?.title).toBe('Gamma');
    expect(summary.best && summary.best.focused_ms).toBe(1_200_000);
  });
  it('returns null average and best without finished sessions', () => {
    const summary = analyticsSummary([], 'week', NOW);
    expect(summary.averageMs).toBeNull();
    expect(summary.best).toBeNull();
  });
});

describe('running sessions and empty data', () => {
  it('includes running-session progress in focus time but not in counts', () => {
    const sessions = [
      makeSession({
        status: 'running',
        started_at: at(19, 9),
        focused_ms: 300_000,
      }),
    ];
    const summary = analyticsSummary(sessions, 'today', NOW);
    expect(summary.focusMs).toBe(300_000);
    expect(summary.finishedCount).toBe(0);
    expect(summary.completedCount).toBe(0);
    expect(summary.hasEligible).toBe(true);
  });
  it('handles empty data without NaN or Infinity', () => {
    const summary = analyticsSummary([], 'week', NOW);
    expect(summary.focusMs).toBe(0);
    expect(summary.completedCount).toBe(0);
    expect(summary.finishedCount).toBe(0);
    expect(summary.daysFocused).toBe(0);
    expect(summary.dayCount).toBe(7);
    expect(summary.averageMs).toBeNull();
    expect(summary.best).toBeNull();
    expect(summary.longestStreakDays).toBe(0);
    expect(summary.breakdown).toEqual({
      elapsedMs: 0,
      distractedMs: 0,
      awayMs: 0,
      unknownMs: 0,
    });
    expect(summary.hasEligible).toBe(false);
    for (const value of Object.values(summary)) {
      expect(value).not.toBeNaN();
    }
  });
});
