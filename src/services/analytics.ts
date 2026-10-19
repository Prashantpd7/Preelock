import type { Session, SessionStatus, Task } from '../types';
export function formatDuration(ms: number) {
  const minutes = Math.floor(ms / 60_000);
  return minutes >= 60
    ? `${Math.floor(minutes / 60)}h ${minutes % 60}m`
    : `${minutes}m`;
}
export function countdown(ms: number) {
  const seconds = Math.max(0, Math.ceil(ms / 1000));
  return `${Math.floor(seconds / 60)
    .toString()
    .padStart(2, '0')}:${(seconds % 60).toString().padStart(2, '0')}`;
}
export function sameDay(a: number, b: number) {
  return new Date(a).toDateString() === new Date(b).toDateString();
}
// Attribute focus to the local date a session started, including sessions crossing midnight.
export function statistics(
  sessions: Session[],
  tasks: Task[],
  now = Date.now(),
) {
  return {
    today: sessions
      .filter((s) => s.started_at !== null && sameDay(s.started_at, now))
      .reduce((sum, s) => sum + s.focused_ms, 0),
    total: sessions.reduce((sum, s) => sum + s.focused_ms, 0),
    completed: sessions.filter((s) => s.status === 'completed').length,
    tasksDone: tasks.filter((t) => t.completed).length,
  };
}
export function week(sessions: Session[], now = new Date()) {
  return Array.from({ length: 7 }, (_, i) => {
    const date = new Date(now);
    date.setDate(date.getDate() - 6 + i);
    return {
      label: date.toLocaleDateString(undefined, { weekday: 'short' }),
      ms: statistics(sessions, [], date.getTime()).today,
    };
  });
}

// Productive metrics deliberately exclude legacy wall-clock sessions.
export function productiveStatistics(sessions: Session[]) {
  const measured = sessions.filter((s) => s.focus?.attention?.enabled);
  const accepted = measured.reduce((n, s) => n + s.focused_ms, 0);
  const elapsed = measured.reduce(
    (n, s) => n + (s.attention?.elapsed_ms ?? 0),
    0,
  );
  return {
    accepted,
    elapsed,
    distracted: measured.reduce(
      (n, s) => n + (s.attention?.distracted_ms ?? 0),
      0,
    ),
    unknown: measured.reduce((n, s) => n + (s.attention?.unknown_ms ?? 0), 0),
    corrected: measured.reduce(
      (n, s) => n + (s.attention?.corrected_ms ?? 0),
      0,
    ),
    percentage: elapsed
      ? Math.min(100, Math.round((accepted / elapsed) * 100))
      : 0,
  };
}

// --- Date-filtered analytics summary ---

export type AnalyticsRange = 'today' | 'week' | 'month' | 'all';

export const ANALYTICS_RANGES: { value: AnalyticsRange; label: string }[] = [
  { value: 'today', label: 'Today' },
  { value: 'week', label: 'Last 7 days' },
  { value: 'month', label: 'This month' },
  { value: 'all', label: 'All time' },
];

// Compact duration for analytics cards. A positive duration is never shown as "0m".
export function formatShortDuration(ms: number): string {
  if (!(ms > 0)) return '0m';
  if (ms < 1000) return '<1s';
  if (ms < 60_000) return `${Math.floor(ms / 1000)}s`;
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${hours}h` : `${hours}h ${rest}m`;
}

// Local calendar day key (YYYY-MM-DD) in the user's timezone.
export function dayKey(date: Date): string {
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${date.getFullYear()}-${month}-${day}`;
}
export function dayKeyOf(ms: number): string {
  return dayKey(new Date(ms));
}
function startOfNextDayMs(ms: number): number {
  const date = new Date(ms);
  date.setHours(24, 0, 0, 0);
  return date.getTime();
}

// Sessions with productive-time tracking. Legacy elapsed-time sessions stay in
// history but are excluded from counted-focus statistics.
export function isMeasured(session: Session): boolean {
  return session.focus?.attention?.enabled === true;
}
const FINISHED_STATUSES: SessionStatus[] = [
  'completed',
  'ended',
  'interrupted',
];
export function isFinished(session: Session): boolean {
  return FINISHED_STATUSES.includes(session.status);
}

// Counted focus time per local calendar day. FOCUSED intervals (and corrected
// portions of other intervals) are split at local midnight; when a session has
// no usable interval history, its focus time counts on its start date.
export function focusedMsByDay(
  sessions: Session[],
  nowMs: number = Date.now(),
): Map<string, number> {
  const byDay = new Map<string, number>();
  const add = (key: string, ms: number) => {
    if (ms > 0) byDay.set(key, (byDay.get(key) ?? 0) + ms);
  };
  for (const session of sessions) {
    if (!isMeasured(session)) continue;
    const total = session.focused_ms;
    if (total <= 0) continue;
    let attributed = 0;
    for (const interval of session.attention?.intervals ?? []) {
      const focusMs =
        interval.state === 'FOCUSED'
          ? interval.duration_ms
          : (interval.corrected_ms ?? 0);
      if (focusMs <= 0 || interval.duration_ms <= 0) continue;
      let cursor = interval.at;
      let remaining = interval.duration_ms;
      while (remaining > 0) {
        const chunk = Math.min(remaining, startOfNextDayMs(cursor) - cursor);
        add(dayKeyOf(cursor), (focusMs * chunk) / interval.duration_ms);
        cursor += chunk;
        remaining -= chunk;
      }
      attributed += focusMs;
    }
    const remainder = total - attributed;
    if (remainder > 0) {
      add(
        session.started_at != null
          ? dayKeyOf(session.started_at)
          : dayKey(new Date(nowMs)),
        remainder,
      );
    }
  }
  return byDay;
}

// Calendar days covered by a range, in the user's local timezone. All time
// starts on the local date of the first tracked session.
export function periodDays(
  range: AnalyticsRange,
  now: Date,
  sessions: Session[] = [],
): Date[] {
  const today = new Date(now);
  today.setHours(0, 0, 0, 0);
  if (range === 'today') return [today];
  if (range === 'week') {
    return Array.from({ length: 7 }, (_, index) => {
      const date = new Date(today);
      date.setDate(date.getDate() - (6 - index));
      return date;
    });
  }
  if (range === 'month') {
    const days: Date[] = [];
    const date = new Date(today.getFullYear(), today.getMonth(), 1);
    while (date <= today) {
      days.push(new Date(date));
      date.setDate(date.getDate() + 1);
    }
    return days;
  }
  let first: number | null = null;
  for (const session of sessions) {
    if (!isMeasured(session) || session.started_at == null) continue;
    if (first == null || session.started_at < first) first = session.started_at;
  }
  const from = new Date(first ?? today.getTime());
  from.setHours(0, 0, 0, 0);
  const days: Date[] = [];
  for (
    let date = new Date(from);
    date <= today;
    date.setDate(date.getDate() + 1)
  ) {
    days.push(new Date(date));
  }
  return days;
}

export interface AnalyticsDay {
  key: string;
  date: Date;
  label: string;
  ms: number;
}
export interface AnalyticsBreakdown {
  elapsedMs: number;
  distractedMs: number;
  awayMs: number;
  unknownMs: number;
}
export interface AnalyticsSummary {
  days: AnalyticsDay[];
  focusMs: number;
  completedCount: number;
  finishedCount: number;
  daysFocused: number;
  dayCount: number;
  averageMs: number | null;
  best: Session | null;
  longestStreakDays: number;
  breakdown: AnalyticsBreakdown;
  hasEligible: boolean;
}

// One consistent set of eligible sessions (productive-time tracked) across all
// cards and the chart. Session counts use the session start date; focus time
// is attributed to the days it happened. Running sessions contribute focus
// progress, but counts, averages and best use finished sessions only.
export function analyticsSummary(
  sessions: Session[],
  range: AnalyticsRange,
  now: Date = new Date(),
): AnalyticsSummary {
  const measured = sessions.filter(isMeasured);
  const days = periodDays(range, now, measured);
  const keys = new Set(days.map(dayKey));
  const byDay = focusedMsByDay(measured, now.getTime());
  const dayList: AnalyticsDay[] = days.map((date) => {
    const key = dayKey(date);
    return {
      key,
      date,
      label: date.toLocaleDateString(undefined, { weekday: 'short' }),
      ms: Math.round(byDay.get(key) ?? 0),
    };
  });
  const inPeriod = (session: Session) =>
    session.started_at != null && keys.has(dayKeyOf(session.started_at));
  const periodSessions = measured.filter(inPeriod);
  const finished = periodSessions.filter(isFinished);
  const completedCount = finished.filter(
    (session) => session.status === 'completed',
  ).length;
  const totalFocused = finished.reduce(
    (sum, session) => sum + session.focused_ms,
    0,
  );
  let best: Session | null = null;
  for (const session of finished) {
    if (!best || session.focused_ms > best.focused_ms) best = session;
  }
  let longestStreakDays = 0;
  let streak = 0;
  for (const day of dayList) {
    streak = day.ms > 0 ? streak + 1 : 0;
    longestStreakDays = Math.max(longestStreakDays, streak);
  }
  const sumField = (pick: (session: Session) => number) =>
    periodSessions.reduce((total, session) => total + pick(session), 0);
  return {
    days: dayList,
    focusMs: dayList.reduce((sum, day) => sum + day.ms, 0),
    completedCount,
    finishedCount: finished.length,
    daysFocused: dayList.filter((day) => day.ms > 0).length,
    dayCount: dayList.length,
    averageMs: finished.length ? totalFocused / finished.length : null,
    best,
    longestStreakDays,
    breakdown: {
      elapsedMs: sumField((s) => s.attention?.elapsed_ms ?? 0),
      distractedMs: sumField((s) => s.attention?.distracted_ms ?? 0),
      awayMs: sumField((s) => s.attention?.absent_ms ?? 0),
      unknownMs: sumField((s) => s.attention?.unknown_ms ?? 0),
    },
    hasEligible: measured.length > 0,
  };
}
