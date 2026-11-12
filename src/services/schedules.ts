import { lockDelayStatus, type Schedule } from '../types';

/** Delay status for a schedule's step 10 change lock, in the
 *  same shape as `lockDelayStatus`. */
export function scheduleLockDelayStatus(schedule: Schedule) {
  return lockDelayStatus(schedule.lock_state);
}

/** Weekday labels in the same 0 = Sunday .. 6 = Saturday order the
 *  backend uses (C `tm_wday`, JS `Date.getDay()`). */
export const DAY_LABELS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

export function formatDays(days: number[]): string {
  const ordered = [...days].sort((a, b) => a - b);
  if (ordered.length === 7) return 'Every day';
  if (ordered.length === 5 && ordered.join(',') === [1, 2, 3, 4, 5].join(','))
    return 'Weekdays';
  if (ordered.length === 2 && ordered.join(',') === [0, 6].join(','))
    return 'Weekends';
  return ordered.map((d) => DAY_LABELS[d] ?? '').join(', ');
}

/** Minutes since midnight for an `HH:MM` string, or null if invalid. */
export function scheduleMinutes(time: string): number | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(time.trim());
  if (!match) return null;
  const hour = Number(match[1]);
  const minute = Number(match[2]);
  if (hour > 23 || minute > 59) return null;
  return hour * 60 + minute;
}

/** Duration of a schedule window in minutes, with whether it runs
 *  overnight. Mirrors `isScheduleActive`: an end earlier than (or
 *  equal to) the start is an overnight window — end equal to start
 *  is a full 24 hours (the form still rejects equal times on
 *  submit; this is only the live display). Null when either time
 *  is invalid. */
export function scheduleWindow(
  startTime: string,
  endTime: string,
): { minutes: number; overnight: boolean } | null {
  const start = scheduleMinutes(startTime);
  const end = scheduleMinutes(endTime);
  if (start === null || end === null) return null;
  if (end > start) return { minutes: end - start, overnight: false };
  return { minutes: 24 * 60 - start + end, overnight: true };
}

/** Short duration label: "45 min", "1 hr", "1 hr 30 min". */
export function formatScheduleDuration(minutes: number): string {
  const hours = Math.floor(minutes / 60);
  const mins = minutes % 60;
  if (hours > 0 && mins > 0) return `${hours} hr ${mins} min`;
  if (hours > 0) return `${hours} hr`;
  return `${mins} min`;
}

/** Total-time text for the schedule forms: the duration, plus
 *  "· Overnight" for windows that cross midnight. "—" when
 *  either time is invalid. */
export function scheduleTotalTime(startTime: string, endTime: string): string {
  const window = scheduleWindow(startTime, endTime);
  if (!window) return '—';
  const text = formatScheduleDuration(window.minutes);
  return window.overnight ? `${text} · Overnight` : text;
}

/** Wall-clock start instant (epoch ms) of the next window start
 *  after `nowMs`, mirroring the backend `next_window_start_ms`
 *  (step 14): the `start_time` on a listed day, for same-day and
 *  overnight windows alike. A start exactly now counts as already
 *  started, never as "next". Null when no day is listed or the
 *  start time is invalid. Day arithmetic assumes 24-hour days, the
 *  same accepted imprecision as the backend. */
export function nextScheduleStartMs(
  schedule: Schedule,
  nowMs: number = Date.now(),
): number | null {
  const start = scheduleMinutes(schedule.start_time);
  if (start === null) return null;
  const now = new Date(nowMs);
  const weekday = now.getDay();
  const minutes = now.getHours() * 60 + now.getMinutes();
  for (let offset = 0; offset <= 7; offset++) {
    const day = (weekday + offset) % 7;
    if (!schedule.days.includes(day)) continue;
    if (offset === 0 && start <= minutes) continue;
    const midnight = new Date(
      now.getFullYear(),
      now.getMonth(),
      now.getDate(),
    ).getTime();
    return midnight + offset * 86_400_000 + start * 60_000;
  }
  return null;
}

/** Mirrors the backend `window_active` exactly: start-inclusive,
 *  end-exclusive, and an end earlier than the start is an overnight
 *  window whose morning half belongs to the previous day's listing. */
export function isScheduleActive(
  schedule: Schedule,
  now: Date = new Date(),
): boolean {
  const start = scheduleMinutes(schedule.start_time);
  const end = scheduleMinutes(schedule.end_time);
  if (start === null || end === null) return false;
  const weekday = now.getDay();
  const minutes = now.getHours() * 60 + now.getMinutes();
  if (end > start) {
    return schedule.days.includes(weekday) && minutes >= start && minutes < end;
  }
  return (
    (schedule.days.includes(weekday) && minutes >= start) ||
    (schedule.days.includes((weekday + 6) % 7) && minutes < end)
  );
}

/** True when the schedule is locked and its window is active right
 *  now: the backend rejects edits, disable and delete in this state,
 *  and the UI disables the enabled toggle for the same state.
 *  Step 10 locks count too, except when a delay request for a
 *  change has already elapsed — then the change is unlocked and
 *  the edit form (which collects the proofs) must stay openable. */
export function isScheduleLockedActive(
  schedule: Schedule,
  now: Date = new Date(),
): boolean {
  if (!schedule.enabled || !isScheduleActive(schedule, now)) return false;
  const lock = schedule.lock;
  const hasGate =
    !!lock &&
    (!!lock.password_hash ||
      (lock.challenge_length ?? 0) > 0 ||
      (lock.delay_minutes ?? 0) > 0);
  if (!schedule.lock_changes && !hasGate) return false;
  const state = schedule.lock_state;
  if (
    state?.delay_for === 'change' &&
    state.delay_until_ms != null &&
    state.delay_until_ms <= now.getTime()
  ) {
    return false;
  }
  return true;
}

/** Client-side form validation, mirroring the backend rules. Empty
 *  string means valid; otherwise the message to show. */
export function validateScheduleForm(input: {
  title: string;
  days: number[];
  start_time: string;
  end_time: string;
}): string {
  if (!input.title.trim() || input.title.trim().length > 160)
    return 'Use a title between 1 and 160 characters.';
  if (input.days.length === 0) return 'Pick at least one day.';
  if (input.days.some((d) => d < 0 || d > 6))
    return 'Days must be Sunday to Saturday.';
  const start = scheduleMinutes(input.start_time);
  const end = scheduleMinutes(input.end_time);
  if (start === null || end === null)
    return 'Use HH:MM times, for example 09:00.';
  if (start === end) return 'Start and end times must differ.';
  return '';
}
