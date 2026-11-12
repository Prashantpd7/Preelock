import type { Session, Snapshot } from '../types';
import type { AnalyticsRange } from './analytics';

/** The four sections shown in the Activity dropdown on the Focus
 *  Sessions page, in display order. */
export type ActivitySection =
  'sessions' | 'schedules' | 'oneshots' | 'allowances';

export const ACTIVITY_SECTIONS: {
  value: ActivitySection;
  label: string;
}[] = [
  { value: 'sessions', label: 'Focus Sessions' },
  { value: 'schedules', label: 'Schedules' },
  { value: 'oneshots', label: 'Start once at' },
  { value: 'allowances', label: 'Allowances' },
];

/** Session statuses that mean a session is still live (see the
 *  Dashboard's isLiveStatus). Live sessions are shown in the active
 *  session area, never in historical Activity. */
export function isLiveSessionStatus(status: string): boolean {
  return status === 'running' || status === 'paused' || status === 'preparing';
}

/** Whether a session was started automatically by a "Start once at"
 *  one-shot timer rather than a manual Focus Session or recurring
 *  Schedule. Checks the persisted `one_shot_started` report event and
 *  falls back to the initial `One-shot block active` enforcement label
 *  in case older events were trimmed on very long sessions. */
export function isOneShotSession(session: Session): boolean {
  if (
    (session.report?.events ?? []).some((e) => e.kind === 'one_shot_started')
  ) {
    return true;
  }
  return session.report?.enforcement === 'One-shot block active';
}

/** Whether a session belongs in historical Activity lists (as opposed
 *  to the live active-session area). */
export function isHistoricalSession(session: Session): boolean {
  return (
    (session.started_at !== null || session.status === 'interrupted') &&
    !isLiveSessionStatus(session.status)
  );
}

/** True while a strict focus session is live (running, paused or
 *  preparing). Browser connection management — disconnecting a
 *  browser, resetting all connections, connecting another browser —
 *  is locked then, and unlocks when the session ends. */
export function hasLiveStrictSession(data: Snapshot): boolean {
  return data.sessions.some(
    (s) => isLiveSessionStatus(s.status) && s.focus?.mode === 'strict',
  );
}

/** Whether an epoch-ms timestamp falls inside a date range. The
 *  Activity toolbar's range filters every section with this one
 *  rule; each section supplies its own date: sessions use
 *  `started_at`, one-shots use their scheduled start
 *  (`fire_at_ms`), and schedules / allowances use `created_at` —
 *  they recur or are ongoing budgets, so no single event date
 *  exists, and `created_at` is the stored date on both models.
 *  'today' is from local midnight, 'week' is today + the previous
 *  6 days, 'month' is the current calendar month, 'all' matches
 *  everything. A null timestamp never matches a bounded range. */
export function inDateRange(
  ms: number | null,
  range: AnalyticsRange,
  now: Date,
): boolean {
  if (range === 'all') return true;
  if (ms == null) return false;
  const start = new Date(ms);
  const today = new Date(now);
  today.setHours(0, 0, 0, 0);
  if (range === 'today') return start >= today;
  if (range === 'week') {
    const weekAgo = new Date(today);
    weekAgo.setDate(weekAgo.getDate() - 6);
    return start >= weekAgo;
  }
  return (
    start.getFullYear() === today.getFullYear() &&
    start.getMonth() === today.getMonth()
  );
}

/** The section the Activity dropdown selects when the Focus Sessions
 *  page first opens. Priority: an active session wins, then existing
 *  schedules, then upcoming one-shot timers, then allowances; with
 *  nothing at all, the session history. */
export function defaultActivitySection(
  data: Snapshot,
  nowMs: number = Date.now(),
): ActivitySection {
  if (data.sessions.some((s) => isLiveSessionStatus(s.status))) {
    return 'sessions';
  }
  if ((data.schedules ?? []).length > 0) return 'schedules';
  if ((data.one_shots ?? []).some((o) => o.fire_at_ms > nowMs)) {
    return 'oneshots';
  }
  if ((data.allowances ?? []).length > 0) return 'allowances';
  return 'sessions';
}
