import type { Session, Task } from '../types';
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
