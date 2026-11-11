import { Link } from 'react-router-dom';
import { Plus } from 'lucide-react';
import { useAppStore } from '../state/store';
import {
  formatDuration,
  formatShortDuration,
  productiveStatistics,
  statistics,
} from '../services/analytics';
import { PageHeading, Stat } from '../components/ui';
import { dayCategoryTotals, localDayStart } from '../services/screenTimeWeek';
export function Dashboard() {
  const { sessions, tasks, preferences, screen_time_usage } = useAppStore(
    (s) => s.data,
  );
  const stats = statistics(sessions, tasks);
  // Today's total screen time, from the always-on usage buckets — the
  // same source the Screen Time page uses. The total is
  // category-independent, so no category overrides are needed.
  const screenTimeTodayMs = dayCategoryTotals(
    screen_time_usage ?? [],
    localDayStart(new Date()),
    'all',
  ).total;
  const productive = productiveStatistics(sessions);
  return (
    <>
      <PageHeading
        eyebrow=""
        title="Make room for deep work."
        description="One intention. One focused session. A little more progress."
        action={
          <Link className="button" to="/sessions">
            <Plus size={17} />
            New session
          </Link>
        }
      />
      {!!preferences.daily_goal && (
        <section className="panel daily-goal">
          <div className="section-heading">
            <h2>Daily goal</h2>
            <strong>
              {formatShortDuration(
                statistics(
                  sessions.filter((s) => s.focus?.attention?.enabled),
                  tasks,
                ).today,
              )}{' '}
              / {formatShortDuration(preferences.daily_goal * 60000)}
            </strong>
          </div>
          <progress
            aria-label="Daily focus goal"
            max={preferences.daily_goal * 60000}
            value={Math.min(
              preferences.daily_goal * 60000,
              statistics(
                sessions.filter((s) => s.focus?.attention?.enabled),
                tasks,
              ).today,
            )}
          />
          <p className="small muted">Based on sessions started today.</p>
        </section>
      )}
      <div className="stats-grid">
        <Stat
          label="Screen Time"
          value={formatDuration(screenTimeTodayMs)}
          detail="Today's total screen time"
        />
        <Stat
          label="Focus time"
          value={formatDuration(productive.accepted)}
          detail="Based on your focus checks"
        />
        <Stat
          label="Total session time"
          value={formatDuration(productive.elapsed)}
          detail="Includes time the timer was paused"
        />
      </div>
      <div className="stats-grid">
        <Stat
          label="Distracted time"
          value={formatDuration(productive.distracted)}
          detail={`${productive.percentage}% focus · ${formatDuration(productive.unknown)} unverified`}
        />
        <Stat
          label="Focus today"
          value={formatDuration(
            statistics(
              sessions.filter((s) => s.focus?.attention?.enabled),
              tasks,
            ).today,
          )}
          detail="Time from sessions started today"
        />
        <Stat
          label="Sessions completed"
          value={String(stats.completed)}
          detail="All-time completed sessions"
        />
      </div>
    </>
  );
}
