/** One-shot start timers (step 6): a single future start time, not a
 *  recurring window. The backend fires the timer only on a tick while
 *  the app is running and nothing else is active; a time that passes
 *  while the app was closed, during a long pause (sleep), or while
 *  another session is running is skipped and the timer is removed,
 *  never fired retroactively. The frontend mirrors only the
 *  client-side checks: a template must be picked and the time must
 *  parse and lie in the future. */

/** Epoch milliseconds for a `datetime-local` input value, or null if
 *  it does not parse. `datetime-local` carries no timezone, so the
 *  browser interprets it in local time — the same local time the
 *  backend's schedule windows use. */
export function oneShotFireAtMs(value: string): number | null {
  if (!value.trim()) return null;
  const ms = new Date(value).getTime();
  return Number.isNaN(ms) ? null : ms;
}

/** Client-side form validation, mirroring the backend rules. Empty
 *  string means valid; otherwise the message to show. */
export function validateOneShotForm(input: {
  title: string;
  templateId: string;
  fireAtMs: number | null;
  nowMs?: number;
}): string {
  if (!input.templateId) return 'Pick a session to use as the template.';
  if (!input.title.trim() || input.title.trim().length > 160)
    return 'Use a title between 1 and 160 characters.';
  if (input.fireAtMs === null) return 'Pick a date and time.';
  if (input.fireAtMs <= (input.nowMs ?? Date.now()))
    return 'Pick a future date and time.';
  return '';
}
