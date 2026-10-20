import { Link } from 'react-router-dom';
import { Plus } from 'lucide-react';
import { useAppStore } from '../state/store';
import {
  countdown,
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
  const isLiveStatus = (s: (typeof sessions)[number]) =>
    ['running', 'paused', 'preparing'].includes(s.status);
  // Every session stays as a reusable card; live ones sort first.
  const upcoming = sessions
    .slice()
    .reverse()
    .sort((a, b) => Number(isLiveStatus(b)) - Number(isLiveStatus(a)));
  const appNames = Object.fromEntries(
    (setup?.applications ?? []).map((a) => [a.id, a.name]),
  );
  const liveSession = sessions.find((s) => isLiveStatus(s));
  const cameraBlocking =
    !!liveSession?.focus?.camera &&
    liveSession.status === 'running' &&
    (liveSession.attention?.state === 'unknown' ||
      liveSession.attention?.state === 'absent');
  return (
    <>
      {liveSession && (
        <section
          className="panel active-session-banner"
          aria-live="polite"
          aria-label="Active session"
        >
          <p className="active-session-kicker">
            <span className="live-dot" aria-hidden="true" />
            Active session · {liveSession.title}
            {liveSession.status === 'preparing' && ' · waiting for browser'}
            {liveSession.status === 'paused' && ' · paused'}
          </p>
          <p className="active-session-timer">
            {countdown(liveSession.remaining_ms)}
          </p>
          <p className="small muted">
            {liveSession.status === 'preparing'
              ? 'Waiting for the browser extension to connect…'
              : 'Distracting apps and websites stay blocked until this session ends.'}
          </p>
          {cameraBlocking && (
            <p className="camera-warn" role="alert">
              Camera isn&apos;t detecting you — timer is paused. Sit in front of
              the camera to resume.
            </p>
          )}
        </section>
      )}
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
