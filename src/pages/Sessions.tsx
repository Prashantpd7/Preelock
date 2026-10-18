import { useState } from 'react';
import { Plus } from 'lucide-react';
import { useAppStore } from '../state/store';
import { desktop } from '../services/api';
import { PageHeading, Empty } from '../components/ui';
import { SessionCard } from '../components/SessionCard';
export function Sessions() {
  const { data, execute, busy } = useAppStore();
  const [title, setTitle] = useState('');
  const [minutes, setMinutes] = useState(data.preferences.default_minutes);
  const [pause, setPause] = useState(true);
  const sessions = data.sessions
    .filter((s) => ['ready', 'running', 'paused'].includes(s.status))
    .reverse();
  return (
    <>
      <PageHeading
        eyebrow="PROTECT YOUR ATTENTION"
        title="Focus Sessions"
        description="Plan a session around one thing that matters."
      />
      <form
        className="panel form-panel"
        onSubmit={async (e) => {
          e.preventDefault();
          if (
            await execute({
              type: 'create_session',
              title,
              minutes,
              allow_pause: pause,
            })
          )
            setTitle('');
        }}
      >
        <h2>Create a session</h2>
        <div className="form-grid">
          <label className="grow">
            Your intention
            <input
              placeholder="What will you focus on?"
              required
              maxLength={160}
              value={title}
              onChange={(e) => setTitle(e.target.value)}
            />
          </label>
          <label>
            Minutes
            <input
              type="number"
              required
              min={1}
              max={480}
              value={minutes}
              onChange={(e) => setMinutes(Number(e.target.value))}
            />
          </label>
        </div>
        <div className="row spread">
          <label className="check-label">
            <input
              type="checkbox"
              checked={pause}
              onChange={(e) => setPause(e.target.checked)}
            />
            Allow pausing
          </label>
          <button disabled={busy || !desktop}>
            <Plus size={16} />
            Create session
          </button>
        </div>
        <p className="small muted">
          No-pause sessions can always be ended. OS restrictions are not active.
        </p>
      </form>
      <div className="section-heading">
        <h2>Planned & active</h2>
        <span className="muted small">One active session at a time</span>
      </div>
      <div className="session-list">
        {sessions.length ? (
          sessions.map((s) => <SessionCard key={s.id} session={s} />)
        ) : (
          <div className="panel">
            <Empty>
              Your next focus session starts with an intention above.
            </Empty>
          </div>
        )}
      </div>
    </>
  );
}
