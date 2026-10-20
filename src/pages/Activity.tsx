import { useState } from 'react';
import { PageHeading, Empty } from '../components/ui';
import { SessionCard } from '../components/SessionCard';
import { useAppStore } from '../state/store';
import { useFocusSetup } from '../hooks/useFocusSetup';
import {
  ANALYTICS_RANGES,
  type AnalyticsRange,
} from '../services/analytics';
function inRange(
  startedAt: number | null,
  range: AnalyticsRange,
  now: Date,
): boolean {
  if (range === 'all') return true;
  if (startedAt == null) return false;
  const start = new Date(startedAt);
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
export function Activity() {
  const { setup, refresh } = useFocusSetup();
  const [range, setRange] = useState<AnalyticsRange>('all');
  const now = new Date();
  const sessions = useAppStore((s) => s.data.sessions)
    .filter((s) => s.started_at !== null || s.status === 'interrupted')
    .filter((s) => inRange(s.started_at, range, now))
    .slice()
    .reverse();
  return (
    <>
      <PageHeading
        eyebrow=""
        title="Activity"
        description="Your past sessions."
      />
      <div className="segmented" role="group" aria-label="Date range">
        {ANALYTICS_RANGES.map((r) => (
          <button
            key={r.value}
            type="button"
            aria-pressed={range === r.value}
            onClick={() => setRange(r.value)}
          >
            {r.label}
          </button>
        ))}
      </div>
      <div className="session-list">
        {sessions.length ? (
          sessions.map((s) => (
            <SessionCard
              key={s.id}
              session={s}
              showStartedAt
              hideStart
              setup={setup}
              onSetupRefresh={refresh}
            />
          ))
        ) : (
          <div className="panel">
            <Empty>
              No activity in this period. Start a focus session to build your
              history.
            </Empty>
          </div>
        )}
      </div>
    </>
  );
}
