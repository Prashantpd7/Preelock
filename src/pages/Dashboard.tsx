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
import { SessionCard } from '../components/SessionCard';
import { useFocusSetup } from '../hooks/useFocusSetup';
export function Dashboard() {
  const { sessions, tasks, preferences } = useAppStore((s) => s.data);
  const { setup, refresh } = useFocusSetup();
  const stats = statistics(sessions, tasks);
  const productive = productiveStatistics(sessions);
  const upcoming = sessions
    .filter((s) =>
      ['ready', 'preparing', 'running', 'paused'].includes(s.status),
    )
    .slice()
    .reverse();
  const appNames = Object.fromEntries(
    (setup?.applications ?? []).map((a) => [a.id, a.name]),
  );
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
          label="Focus time"
          value={formatDuration(productive.accepted)}
          detail="Based on your focus checks"
        />
        <Stat
          label="Total session time"
          value={formatDuration(productive.elapsed)}
          detail="Includes time the timer was paused"
        />
        <Stat
          label="Distracted time"
          value={formatDuration(productive.distracted)}
          detail={`${productive.percentage}% focus · ${formatDuration(productive.unknown)} unverified`}
        />
      </div>
      <div className="stats-grid">
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
        <Stat
          label="Task progress"
          value={`${stats.tasksDone} / ${tasks.length}`}
          detail="Completed in your workspace"
        />
      </div>
      {!!upcoming.length && (
        <>
          <div className="section-heading">
            <h2>Your sessions</h2>
          </div>
          <div className="session-list">
            {upcoming.map((s) => (
              <SessionCard
                session={s}
                key={s.id}
                appNames={appNames}
                setup={setup}
                onSetupRefresh={refresh}
              />
            ))}
          </div>
        </>
      )}
    </>
  );
}
