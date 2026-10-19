import { useEffect, useState } from 'react';
import { desktop } from '../services/api';
import { settingsAction, type SettingsInfo } from '../services/settings';
import { countdown } from '../services/analytics';
export function BreakBar() {
  const [info, setInfo] = useState<SettingsInfo>({});
  const [error, setError] = useState('');
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
            : 'Ready for a short break?'}
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
          </>
        )}
      </div>
    </aside>
  );
}
