import { useState } from 'react';
import { Pause, Play, Square, Timer } from 'lucide-react';
import { useAppStore } from '../state/store';
import { countdown, formatDuration } from '../services/analytics';
import { desktop } from '../services/api';
import type { Session } from '../types';
export function SessionCard({ session }: { session: Session }) {
  const execute = useAppStore((s) => s.execute);
  const busy = useAppStore((s) => s.busy);
  const [recovery, setRecovery] = useState(false);
  const [confirmation, setConfirmation] = useState('');
  const live = ['running', 'paused', 'preparing'].includes(session.status);
  const committed = session.focus && session.focus.mode !== 'standard';
  const report = session.report;
  const attention = session.attention;
  const [intervalId, setIntervalId] = useState('');
  const [note, setNote] = useState('');
  const reviewable =
    attention?.intervals.filter(
      (i) => !['FOCUSED', 'RECOVERING'].includes(i.state) && !i.corrected_ms,
    ) ?? [];
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
            {session.focus?.mode ?? 'standard'} ·{' '}
            {committed
              ? 'Emergency exit available'
              : session.allow_pause
                ? 'Flexible'
                : 'No pause'}
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
      {session.focus?.attention?.enabled && attention && (
        <section
          className="session-report"
          aria-label="Productive-time tracking"
        >
          <p>
            <strong>
              {session.status === 'paused' ? 'PAUSED BY YOU' : attention.state}
            </strong>{' '}
            · {session.focus.attention.verification.replaceAll('_', ' ')}
          </p>
          <p role="status">{attention.reason || 'Waiting to start'}</p>
          <div className="report-metrics">
            <span>Accepted {countdown(session.focused_ms)}</span>
            <span>Elapsed {countdown(attention.elapsed_ms)}</span>
            <span>Distracted {countdown(attention.distracted_ms)}</span>
            <span>Absent {countdown(attention.absent_ms)}</span>
            <span>Unknown / gaps {countdown(attention.unknown_ms)}</span>
            <span>User corrections {countdown(attention.corrected_ms)}</span>
          </div>
          <p className="muted small">
            Restrictions stay active during attention pauses. Accepted time is
            an estimate under your selected policy, not proof of learning.
            Elapsed includes monitoring gaps; explicit session pauses are
            excluded. Original classification totals retain corrected intervals.
          </p>
          {session.status === 'running' &&
            session.focus.attention.verification === 'manual_checkpoints' && (
              <button
                disabled={busy}
                onClick={() =>
                  void execute({ type: 'confirm_work', id: session.id })
                }
              >
                I am working · accept next 60 seconds
              </button>
            )}
          <details className="event-history">
            <summary>Attention timeline & corrections</summary>
            <ul>
              {attention.intervals
                .slice(-30)
                .reverse()
                .map((i) => (
                  <li key={i.id}>
                    <time>{new Date(i.at).toLocaleTimeString()}</time>
                    <span>
                      {i.state} · {countdown(i.duration_ms)} · {i.reason}
                      {i.corrected_ms > 0
                        ? ` · User accepted ${countdown(i.corrected_ms)}: ${i.correction_note}`
                        : ''}
                    </span>
                  </li>
                ))}
            </ul>
            {reviewable.length > 0 && session.status !== 'completed' && (
              <form
                onSubmit={async (e) => {
                  e.preventDefault();
                  if (
                    await execute({
                      type: 'correct_interval',
                      id: session.id,
                      interval_id: intervalId,
                      note,
                    })
                  ) {
                    setNote('');
                    setIntervalId('');
                  }
                }}
              >
                <p className="muted small">
                  Correct a recent interval you know was productive. Corrections
                  are labeled self-report and cannot credit sleep/restart gaps.
                  Review window: latest 300 intervals.
                </p>
                <label>
                  Interval
                  <select
                    required
                    value={intervalId}
                    onChange={(e) => setIntervalId(e.target.value)}
                  >
                    <option value="">Choose interval</option>
                    {reviewable
                      .slice()
                      .reverse()
                      .map((i) => (
                        <option key={i.id} value={i.id}>
                          {new Date(i.at).toLocaleTimeString()} · {i.state} ·{' '}
                          {countdown(i.duration_ms)}
                        </option>
                      ))}
                  </select>
                </label>
                <label>
                  Why was this productive?
                  <input
                    required
                    minLength={3}
                    maxLength={240}
                    value={note}
                    onChange={(e) => setNote(e.target.value)}
                  />
                </label>
                <button disabled={busy || !intervalId}>
                  Accept reviewed interval
                </button>
              </form>
            )}
          </details>
        </section>
      )}
      {!session.focus?.attention?.enabled && (
        <p className="muted small">
          Legacy elapsed-time session; not verified productive time.
        </p>
      )}
      {session.status === 'preparing' && (
        <p role="status" className="notice">
          Waiting for the paired extension to install rules. Use Sync now in its
          popup. The timer starts only after acknowledgement; setup times out
          after 45 seconds.
        </p>
      )}
      {report && (
        <div className="session-report">
          <p className="small">
            {report.enforcement ||
              'Restrictions activate when the session starts.'}
          </p>
          {session.focus?.activity_consent && (
            <>
              <p className="muted small">
                {live
                  ? `Activity monitoring: ${session.status === 'running' ? 'on' : 'off'}`
                  : 'Session activity summary'}{' '}
                · {report.active_app || 'No current app sample'}
              </p>
              <div className="report-metrics">
                <span>Approved {formatDuration(report.approved_ms)}</span>
                <span>Distracting {formatDuration(report.distracting_ms)}</span>
                <span>Idle {formatDuration(report.idle_ms)}</span>
                <span>{report.interruptions} interruptions</span>
              </div>
            </>
          )}
          <p className="muted small">
            Observed browser blocks: {report.browser_blocks} (unpacked-extension
            feedback only)
          </p>
          {(session.focus?.camera ||
            (report.camera_status && report.camera_status !== 'off')) && (
            <p role="status">
              Camera: {report.camera_status || 'Waiting for session activation'}{' '}
              · estimates may be wrong.
            </p>
          )}
        </div>
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
        {session.status === 'running' && session.allow_pause && !committed && (
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
        {(session.status === 'ready' || (live && !committed)) && (
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
        {live && (
          <button className="secondary" onClick={() => setRecovery(!recovery)}>
            Emergency exit
          </button>
        )}
        {session.focus?.camera && live && (
          <button
            className="secondary"
            disabled={busy}
            onClick={() =>
              void execute({ type: 'disable_camera', id: session.id })
            }
          >
            Stop camera
          </button>
        )}
        <span className="muted small">
          {formatDuration(session.focused_ms)} focused
        </span>
      </div>
      {recovery && live && (
        <form
          className="recovery"
          onSubmit={async (e) => {
            e.preventDefault();
            if (
              await execute({
                type: 'emergency_exit',
                id: session.id,
                confirmation,
              })
            ) {
              setRecovery(false);
              setConfirmation('');
            }
          }}
        >
          <p>
            Emergency exit releases restrictions and records an interruption in
            your history. Browser cleanup requires the extension to sync; its
            popup also has independent recovery.
          </p>
          <label>
            Type END FOCUS
            <input
              value={confirmation}
              onChange={(e) => setConfirmation(e.target.value)}
              autoComplete="off"
            />
          </label>
          <button disabled={busy || confirmation !== 'END FOCUS'}>
            Confirm emergency exit
          </button>
        </form>
      )}
      {report?.events.length ? (
        <details className="event-history">
          <summary>
            Distraction & recovery history ({report.events.length})
          </summary>
          <ul>
            {report.events
              .slice()
              .reverse()
              .map((event, index) => (
                <li key={index}>
                  <time>{new Date(event.at).toLocaleTimeString()}</time>
                  <span>{event.detail}</span>
                </li>
              ))}
          </ul>
        </details>
      ) : null}
    </article>
  );
}
