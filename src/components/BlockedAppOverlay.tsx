import { useEffect, useState } from 'react';
import { X } from 'lucide-react';
import { useApplication } from '../hooks/useApplication';
import { countdown } from '../services/analytics';
import { api } from '../services/api';
import { useAppStore } from '../state/store';

/** "Why am I blocked" app page, rendered in its own compact,
 *  always-on-top Tauri popup window (label `blocked-app`, see `blockedpage.rs`).
 *  The engine records the blocked app + its true reason when it
 *  hides the app (throttled); this small popup shows it instead of the
 *  app silently vanishing without taking over the entire screen.
 *
 *  Unlike the frozen overlay this popup is purely informational and
 *  trivially dismissible: the close cross button, "Back to work", and the Escape
 *  key all clear the notice via `dismiss_blocked_app_page`, and
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
    document.documentElement.classList.add('in-blocked-popup');
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => {
      document.documentElement.classList.remove('in-blocked-popup');
      window.clearInterval(timer);
    };
  }, []);

  const dismiss = () => {
    void api.dismissBlockedAppPage();
    void import('@tauri-apps/api/window')
      .then(({ getCurrentWindow }) => {
        try {
          const win = getCurrentWindow();
          if (win.label === 'blocked-app') {
            void win.close();
          }
        } catch {
          // Ignore
        }
      })
      .catch(() => {});
  };

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') dismiss();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
  if (!notice) {
    return (
      <main className="blocked-app-popup">
        <div className="blocked-app-header">
          <span className="frozen-kicker">Preelock · Blocked</span>
          <button
            type="button"
            className="icon-button"
            aria-label="Close dialog"
            onClick={dismiss}
          >
            <X size={18} />
          </button>
        </div>
        <div className="blocked-app-body">
          <h1>Nothing blocked</h1>
          <p className="muted">This window closes on its own.</p>
        </div>
        <div className="blocked-app-footer">
          <button
            type="button"
            className="blocked-app-dismiss"
            onClick={dismiss}
          >
            Back to work
          </button>
        </div>
      </main>
    );
  }
  const endsAt = notice.ends_at_ms ?? null;
  const left = endsAt != null ? Math.max(0, endsAt - now) : null;
  return (
    <main className="blocked-app-popup">
      <div className="blocked-app-header">
        <span className="frozen-kicker">Preelock · Blocked</span>
        <button
          type="button"
          className="icon-button"
          aria-label="Close dialog"
          onClick={dismiss}
        >
          <X size={18} />
        </button>
      </div>
      <div className="blocked-app-body">
        <h1>Stay with your task</h1>
        <p className="blocked-app-reason">{notice.reason}</p>
        {left != null && (
          <p className="frozen-timer" aria-label="Time remaining">
            {countdown(left)}
          </p>
        )}
      </div>
      <div className="blocked-app-footer">
        <button type="button" className="blocked-app-dismiss" onClick={dismiss}>
          Back to work
        </button>
        <p className="small muted">
          {notice.reason.toLowerCase().includes('website') ||
          notice.app_name.includes('.')
            ? 'The website stays blocked — this only closes the popup. You can also press Esc.'
            : 'The app stays blocked — this only closes the popup. You can also press Esc.'}
        </p>
      </div>
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
