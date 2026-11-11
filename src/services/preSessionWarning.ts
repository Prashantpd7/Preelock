import type { FocusConfig, Session, Snapshot } from '../types';
import { nextScheduleStartMs } from './schedules';

/** How far ahead the pre-session warning looks: a scheduled or
 *  one-shot session starting within this window gets the in-app
 *  popup (and, by default, the 5-minute OS notification — the
 *  default `warning_minutes` is "5"). The popup does not consult
 *  the notification preference: it is the fallback when OS
 *  notifications are off. */
export const PRE_SESSION_WINDOW_MS = 5 * 60_000;

/** One auto-start the warning can fire for. */
export interface UpcomingStart {
  kind: 'schedule' | 'one_shot';
  id: string;
  title: string;
  startMs: number;
  focus: FocusConfig;
}

function isLive(session: Session): boolean {
  return ['running', 'paused', 'preparing'].includes(session.status);
}

/** Scheduled and one-shot sessions starting within `windowMs`,
 *  soonest first. Schedules only count when enabled, when their
 *  next start is still in the future, and when no session from
 *  that schedule is already live (a live session means the block
 *  already started — that is the at-start case, below). A manual
 *  session running does not hide a schedule: the schedule still
 *  starts when the manual session ends, if still in its window,
 *  so the warning still fires — the same honest edge the OS
 *  advance warnings already have. */
export function upcomingStarts(
  data: Snapshot,
  nowMs: number,
  windowMs: number = PRE_SESSION_WINDOW_MS,
): UpcomingStart[] {
  const out: UpcomingStart[] = [];
  const liveScheduleIds = new Set(
    data.sessions
      .filter(isLive)
      .map((s) => s.schedule_id)
      .filter((id): id is string => !!id),
  );
  for (const schedule of data.schedules ?? []) {
    if (!schedule.enabled) continue;
    if (liveScheduleIds.has(schedule.id)) continue;
    const startMs = nextScheduleStartMs(schedule, nowMs);
    if (startMs === null || startMs <= nowMs) continue;
    if (startMs - nowMs > windowMs) continue;
    out.push({
      kind: 'schedule',
      id: schedule.id,
      title: schedule.title,
      startMs,
      focus: schedule.focus,
    });
  }
  for (const shot of data.one_shots ?? []) {
    if (shot.fire_at_ms <= nowMs) continue;
    if (shot.fire_at_ms - nowMs > windowMs) continue;
    out.push({
      kind: 'one_shot',
      id: shot.id,
      title: shot.title,
      startMs: shot.fire_at_ms,
      focus: shot.focus,
    });
  }
  out.sort((a, b) => a.startMs - b.startMs);
  return out;
}

/** Live sessions that were auto-started by a schedule or a
 *  one-shot timer: schedule sessions carry their `schedule_id`,
 *  one-shot sessions carry a `one_shot_started` report event (and
 *  schedule sessions a `schedule_started` one). Manual sessions
 *  never match, so the at-start popup never fires for a session
 *  the user started by hand — those already went through the
 *  "Before you start" readiness dialog. */
export function autoStartedLiveSessions(data: Snapshot): Session[] {
  return data.sessions.filter((s) => {
    if (!['running', 'preparing'].includes(s.status)) return false;
    if (s.schedule_id) return true;
    return (s.report?.events ?? []).some(
      (e) => e.kind === 'schedule_started' || e.kind === 'one_shot_started',
    );
  });
}

/** Whole minutes until a start, rounded UP (30 seconds left still
 *  reads as 1 minute), for the popup text. Only called with a
 *  future start, so the result is at least 1. */
export function minutesUntilStart(startMs: number, nowMs: number): number {
  return Math.max(1, Math.ceil((startMs - nowMs) / 60_000));
}
