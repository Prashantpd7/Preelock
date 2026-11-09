import { useEffect, useState } from 'react';
import { useApplication } from '../hooks/useApplication';
import { countdown } from '../services/analytics';
import { api } from '../services/api';
import { useAppStore } from '../state/store';

/** "Why am I blocked" app page, rendered in its own fullscreen,
 *  always-on-top Tauri window (label `blocked-app`, see `lib.rs`).
 *  The engine records the blocked app + its true reason when it
 *  hides the app (throttled); this page shows it instead of the
 *  app just silently vanishing — like the website blocked page.
 *
 *  Unlike the frozen overlay this page is purely informational and
 *  trivially dismissible: the "Back to work" button and the Escape
 *  key both clear the notice via `dismiss_blocked_app_page`, and
 *  the tick sync destroys the window. The block itself is
 *  untouched. A stale notice (older than 2 minutes) never shows
 *  the page: the sync treats it as gone.
 */
export function BlockedAppOverlay() {
  const notice = useAppStore((s) => s.data.blocked_app_notice);
  // Live countdown for timed locks, ticking in state like the
  // frozen overlay does (the snapshot refreshes every second).
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') void api.dismissBlockedAppPage();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
  if (!notice) {
    return (
      <main className="frozen-overlay">
        <p className="frozen-kicker">Preelock</p>
        <h1>Nothing blocked</h1>
        <p className="muted">This window closes on its own.</p>
      </main>
    );
  }
  const endsAt = notice.ends_at_ms ?? null;
  const left = endsAt != null ? Math.max(0, endsAt - now) : null;
  return (
    <main className="frozen-overlay">
      <p className="frozen-kicker">Preelock · Blocked</p>
      <h1>Stay with your task</h1>
      <p className="blocked-app-reason">{notice.reason}</p>
      {left != null && (
        <p className="frozen-timer" aria-label="Time remaining">
          {countdown(left)}
        </p>
      )}
      <button
        type="button"
        className="blocked-app-dismiss"
        onClick={() => void api.dismissBlockedAppPage()}
      >
        Back to work
      </button>
      <p className="small muted">
        The app stays blocked — this only closes the page. You can also press
        Esc.
      </p>
    </main>
  );
}

/** The overlay window's app shell: polls the snapshot like the
 *  main app does, so a newer block replaces the page content and a
 *  dismissed notice closes the window. Used only when the current
 *  Tauri window's label is `blocked-app` (see `App.tsx`). */
export function BlockedAppOverlayWindow() {
  useApplication();
  return <BlockedAppOverlay />;
}
