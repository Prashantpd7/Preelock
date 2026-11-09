import { useEffect, useState } from 'react';
import { useApplication } from '../hooks/useApplication';
import { countdown } from '../services/analytics';
import { useAppStore } from '../state/store';
import { frozenLabel } from '../types';

/** Step 12 Frozen Turkey overlay, rendered in its own fullscreen,
 *  always-on-top Tauri window (label `frozen`, see `lib.rs`).
 *  It has NO close affordance and NO unlock button, by design:
 *  the lock ends only when its timer runs out, at which point the
 *  backend destroys this window. The overlay is the visible lock
 *  only — the engine's app + website blocking is the real
 *  enforcement, so switching Spaces on macOS or killing the app
 *  on Windows does not defeat the block (see `frozen.rs`).
 *
 *  The timer shown is the wall-clock time left: a frozen session
 *  expires on wall-clock time even if no productive time was
 *  recorded, so the productive `remaining_ms` alone would overstate
 *  it. The shown value is the smaller of the two.
 */
export function FrozenOverlay() {
  const data = useAppStore((s) => s.data);
  // Wall-clock countdown ticks in state, not in render: the
  // snapshot refreshes every second in the overlay window, and
  // this timer keeps the displayed seconds moving between polls.
  const [now, setNow] = useState(0);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  const session = data.sessions.find(
    (s) => s.focus?.frozen?.enabled && s.status === 'running',
  );
  if (!session) {
    return (
      <main className="frozen-overlay">
        <p className="frozen-kicker">Preelock · Frozen</p>
        <h1>Frozen block ended</h1>
        <p className="muted">
          The timer ran out. This window closes on its own.
        </p>
      </main>
    );
  }
  const wallLeft =
    session.started_at != null && now > 0
      ? session.started_at + session.duration_minutes * 60_000 - now
      : session.remaining_ms;
  const left = Math.max(0, Math.min(session.remaining_ms, wallLeft));
  const label = frozenLabel(session.focus);
  return (
    <main className="frozen-overlay">
      <p className="frozen-kicker">Preelock · Frozen</p>
      <h1>{session.title}</h1>
      <p className="frozen-timer" aria-label="Time remaining">
        {countdown(left)}
      </p>
      <p className="muted">
        Everything except what you allowed is blocked until this
        timer runs out. There is no way to end it early — that is
        the point of Frozen.
      </p>
      {label && <p className="small">{label}</p>}
      <p className="small muted">
        On macOS you can switch Spaces away from this screen, but
        the block stays on. Closing or killing the app does not end
        it either: it comes back on relaunch until the timer ends.
      </p>
    </main>
  );
}

/** The overlay window's app shell: polls the snapshot like the
 *  main app does, so the timer counts down and the window's
 *  content goes away with the session. Used only when the current
 *  Tauri window's label is `frozen` (see `App.tsx`). */
export function FrozenOverlayWindow() {
  useApplication();
  return <FrozenOverlay />;
}
