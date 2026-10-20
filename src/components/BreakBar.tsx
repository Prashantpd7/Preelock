import { useEffect, useState } from 'react';
import { desktop } from '../services/api';
import { settingsAction, type SettingsInfo } from '../services/settings';
import { countdown } from '../services/analytics';
import { useAppStore } from '../state/store';
export function BreakBar() {
  const [info, setInfo] = useState<SettingsInfo>({});
  const [error, setError] = useState('');
  const sessions = useAppStore((s) => s.data.sessions);
  const execute = useAppStore((s) => s.execute);
  const busy = useAppStore((s) => s.busy);
  const lastFinished =
    sessions.find((s) => s.id === info.break_offer) ??
    sessions
      .filter((s) => ['completed', 'ended', 'interrupted'].includes(s.status))
      .sort((a, b) => (b.ended_at ?? 0) - (a.ended_at ?? 0))[0];
  useEffect(() => {
    if (!desktop) return;
    let active = true;
    const poll = () =>
      void settingsAction({ type: 'status' })
        .then((v) => {
          if (active) setInfo(v);
        })
        .catch(() => {});
    poll();
    const timer = window.setInterval(poll, 1000);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, []);
  async function act(type: 'start_break' | 'end_break' | 'dismiss_break') {
    try {
      await settingsAction({ type });
      setInfo(await settingsAction({ type: 'status' }));
      setError('');
    } catch (e) {
      setError(String(e));
    }
  }
  async function startAgain() {
    if (!lastFinished) return;
    try {
      await settingsAction({ type: 'dismiss_break' });
      setInfo(await settingsAction({ type: 'status' }));
      if (await execute({ type: 'restart_session', id: lastFinished.id })) {
        await execute({ type: 'start_session', id: lastFinished.id });
      }
      setError('');
    } catch (e) {
      setError(String(e));
    }
  }
  if (!info.break_remaining && !info.break_offer) return null;
  return (
    <aside className="break-bar" aria-label="Break timer">
      <div>
        <strong>
          {info.break_remaining
            ? `Break · ${countdown(info.break_remaining)}`
            : 'Focus complete'}
        </strong>
        <p className="small muted">
          {info.break_remaining
            ? 'Your next session starts when you are ready.'
            : 'Take a break or start again.'}
        </p>
        {error && <p role="alert">{error}</p>}
      </div>
      <div className="row controls">
        {info.break_remaining ? (
          <button className="secondary" onClick={() => void act('end_break')}>
            End break
          </button>
        ) : (
          <>
            <button
              className="secondary"
              onClick={() => void act('dismiss_break')}
            >
              Not now
            </button>
            <button onClick={() => void act('start_break')}>Start break</button>
            {lastFinished && (
              <button disabled={busy} onClick={() => void startAgain()}>
                Start again
              </button>
            )}
          </>
        )}
      </div>
    </aside>
  );
}
