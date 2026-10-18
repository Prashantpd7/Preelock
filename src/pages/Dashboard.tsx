import { Link } from 'react-router-dom';
import { ArrowRight, Plus, Sparkles } from 'lucide-react';
import { useAppStore } from '../state/store';
import { formatDuration, statistics } from '../services/analytics';
import { Empty, PageHeading, Stat } from '../components/ui';
import { SessionCard } from '../components/SessionCard';
import { TaskList } from '../components/TaskList';
export function Dashboard() {
  const { sessions, tasks } = useAppStore((s) => s.data);
  const stats = statistics(sessions, tasks);
  const active = sessions.find((s) => ['running', 'paused'].includes(s.status));
  const recent = sessions
    .filter((s) => ['ended', 'completed'].includes(s.status))
    .slice(-3)
    .reverse();
  return (
    <>
      <PageHeading
        eyebrow="A LITTLE CLARITY GOES A LONG WAY"
        title="Make room for deep work."
        description="One intention. One focused session. A little more progress."
        action={
          <Link className="button" to="/sessions">
            <Plus size={17} />
            New session
          </Link>
        }
      />
      <div className="stats-grid">
        <Stat
          label="Focus today"
          value={formatDuration(stats.today)}
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
      <div className="dashboard-grid">
        <section>
          <div className="section-heading">
            <h2>Your focus space</h2>
            <span className="eyebrow">
              {active ? 'IN PROGRESS' : 'READY WHEN YOU ARE'}
            </span>
          </div>
          {active ? (
            <SessionCard session={active} />
          ) : (
            <div className="panel focus-empty">
              <span className="focus-orbit">
                <Sparkles size={30} />
              </span>
              <h2>Start with a single intention.</h2>
              <p className="muted">
                Choose what matters, set your time,
                <br />
                and give it your full attention.
              </p>
              <Link className="button" to="/sessions">
                Create a focus session <ArrowRight size={16} />
              </Link>
              <small>No blocks or monitoring. Just space to focus.</small>
            </div>
          )}
        </section>
        <section>
          <div className="section-heading">
            <h2>On your list</h2>
            <Link to="/tasks">
              View all <ArrowRight size={14} />
            </Link>
          </div>
          <div className="panel">
            <TaskList tasks={tasks.filter((t) => !t.completed).slice(0, 5)} />
          </div>
          <div className="quiet-note">
            <span className="eyebrow">A SMALL REMINDER</span>
            <p>
              You don’t need to do it all.
              <br />
              Just the next meaningful thing.
            </p>
          </div>
        </section>
      </div>
      <section>
        <div className="section-heading">
          <h2>Recent sessions</h2>
          <Link to="/activity">
            View history <ArrowRight size={14} />
          </Link>
        </div>
        <div className="panel">
          {recent.length ? (
            recent.map((s) => (
              <div className="history-row" key={s.id}>
                <span className="grow">{s.title}</span>
                <span className="muted">{formatDuration(s.focused_ms)}</span>
                <span className="badge">{s.status}</span>
              </div>
            ))
          ) : (
            <Empty>
              Your session history will appear here after your first session.
            </Empty>
          )}
        </div>
      </section>
    </>
  );
}
