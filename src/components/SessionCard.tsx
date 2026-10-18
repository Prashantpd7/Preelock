import { Pause, Play, Square, Timer } from 'lucide-react';
import { useAppStore } from '../state/store';
import { countdown, formatDuration } from '../services/analytics';
import { desktop } from '../services/api';
import type { Session } from '../types';
export function SessionCard({ session }: { session: Session }) {
  const execute = useAppStore((s) => s.execute);
  const busy = useAppStore((s) => s.busy);
  const live = session.status === 'running' || session.status === 'paused';
  return (
    <article className={`panel session-card ${live ? 'active-session' : ''}`}>
      <div className="row">
        <span className="icon-tile">
          <Timer size={20} />
        </span>
        <div className="grow">
          <h3>{session.title}</h3>
          <p className="muted">
            {session.duration_minutes} minutes ·{' '}
            {session.allow_pause ? 'Flexible' : 'No pause'}
            {!session.allow_pause && ' · ending is always available'}
          </p>
        </div>
        <span
          className={`badge ${session.status === 'running' ? 'green' : ''}`}
        >
          {session.status}
        </span>
      </div>
      {live && (
        <>
          <div className="timer" aria-label="Time remaining">
            {countdown(session.remaining_ms)}
          </div>
          <progress
            aria-label="Session progress"
            max={session.duration_minutes * 60_000}
            value={session.focused_ms}
          />
        </>
      )}
      <div className="row controls">
        {session.status === 'ready' && (
          <button
            disabled={busy || !desktop}
            onClick={() =>
              void execute({ type: 'start_session', id: session.id })
            }
          >
            <Play size={15} />
            Start session
          </button>
        )}
        {session.status === 'running' && session.allow_pause && (
          <button
            className="secondary"
            disabled={busy}
            onClick={() =>
              void execute({ type: 'pause_session', id: session.id })
            }
          >
            <Pause size={15} />
            Pause
          </button>
        )}
        {session.status === 'paused' && (
          <button
            disabled={busy}
            onClick={() =>
              void execute({ type: 'resume_session', id: session.id })
            }
          >
            <Play size={15} />
            Resume
          </button>
        )}
        {(live || session.status === 'ready') && (
          <button
            className="secondary"
            disabled={busy}
            onClick={() =>
              void execute({ type: 'end_session', id: session.id })
            }
          >
            <Square size={14} />
            {session.status === 'ready' ? 'Cancel' : 'End session'}
          </button>
        )}
        <span className="muted small">
          {formatDuration(session.focused_ms)} focused
        </span>
      </div>
    </article>
  );
}
