import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { useAppStore } from '../state/store';
import { countdown } from '../services/analytics';
import { liveSession, sessionNeedsBrowser } from '../services/activeSession';
import { ActiveSessionDetailsModal } from './ActiveSessionDetailsModal';

/** Global active-session card, mounted once in AppLayout so it
 *  appears at the top of every main page. This is the Dashboard's
 *  banner moved here unchanged — same content, same live data from
 *  the shared store (which refreshes from the backend every
 *  second), plus click-to-open details. The whole card opens the
 *  details popup; the Connect browser links inside it navigate
 *  instead (they stop the click from bubbling). */
export function ActiveSessionBanner() {
  const { sessions, browser } = useAppStore((s) => s.data);
  const session = liveSession(sessions);
  const [detailsOpen, setDetailsOpen] = useState(false);
  // Step 14 grace: a running session with a future
  // grace_until_ms has not started enforcing yet. The clock
  // lives in an effect (never in render, per the purity rule)
  // and ticks every second while grace is set, so the countdown
  // below stays live even between snapshot refreshes.
  const graceUntilMs = session?.grace_until_ms ?? null;
  // The popup only ever shows a live session: reset its open state
  // during render when the session ends, so a later session never
  // auto-opens it. (Render-phase adjustment, per the React docs —
  // not an effect — so it does not cascade renders.)
  if (!session && detailsOpen) setDetailsOpen(false);
  // The clock ticks from an effect, never from render (purity
  // rule): an immediate timeout syncs it the moment grace starts
  // (a mount-time initializer would go stale if the banner mounted
  // long before grace began), and the interval keeps it ticking.
  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    if (!graceUntilMs) return;
    const immediate = window.setTimeout(() => setNowMs(Date.now()), 0);
    const timer = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => {
      window.clearTimeout(immediate);
      window.clearInterval(timer);
    };
  }, [graceUntilMs]);
  const graceRemainingSecs =
    session?.status === 'running' && graceUntilMs
      ? Math.max(0, Math.ceil((graceUntilMs - nowMs) / 1000))
      : 0;
  const inGrace = graceRemainingSecs > 0;
  if (!session) return null;
  // Does this session use browser website blocking? Mirrors the Rust
  // FocusConfig::network_required() logic.
  const needsBrowser = sessionNeedsBrowser(session.focus);
  // Show the reconnect button when the live session needs the browser
  // but the extension is not connected. The session keeps running and
  // apps stay blocked; only website blocking is paused.
  // Step 11 fail-closed: the live session's own flag is authoritative
  // (the shared browser status can show another browser connected).
  // A fail-closed session gets its own banner below, so the generic
  // transient-disconnect banner is suppressed for it.
  const showFailClosed =
    session.status === 'running' && !inGrace && !!session.browser_fail_closed;
  const showReconnectBrowser =
    session.status === 'running' &&
    !inGrace &&
    needsBrowser &&
    !browser?.connected &&
    !showFailClosed;
  const cameraBlocking =
    !!session.focus?.camera &&
    session.status === 'running' &&
    !inGrace &&
    (session.attention?.state === 'unknown' ||
      session.attention?.state === 'absent');
  const openDetails = () => setDetailsOpen(true);
  return (
    <>
      <section
        className="panel active-session-banner clickable"
        aria-live="polite"
        aria-label={`Active session: ${session.title}. Activate for details.`}
        role="button"
        tabIndex={0}
        onClick={openDetails}
        onKeyDown={(e) => {
          if (
            e.target !== e.currentTarget &&
            e.target instanceof HTMLElement &&
            e.target.closest('a,button,input,select,textarea,[role="button"]')
          ) {
            return;
          }
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            openDetails();
          }
        }}
      >
        <p className="active-session-kicker">
          <span className="live-dot" aria-hidden="true" />
          Active session · {session.title}
          {session.status === 'preparing' && ' · waiting for browser'}
          {session.status === 'paused' && ' · paused'}
          {inGrace && ' · starting soon'}
        </p>
        <p className="active-session-timer">
          {inGrace
            ? `Starts in ${graceRemainingSecs}s`
            : countdown(session.remaining_ms)}
        </p>
        <p className="small muted">
          {inGrace
            ? 'Blocking begins when the countdown ends. Finish up — nothing is blocked yet.'
            : session.status === 'preparing'
              ? 'Waiting for the browser extension to connect…'
              : 'Distracting apps and websites stay blocked until this session ends.'}
        </p>
        {showFailClosed && (
          <p>
            <Link
              className="button secondary"
              to="/settings?tab=Connections"
              aria-label="Connect browser from Settings Connections"
              onClick={(e) => e.stopPropagation()}
            >
              Connect browser
            </Link>
            <span className="small muted" style={{ marginLeft: '0.75rem' }}>
              Browser blocked until the extension is connected — this
              session&apos;s website rules were never applied, so the browser
              app itself stays blocked. Killing the app or removing the
              extension cannot end this block early; it resumes when Preelock
              restarts. This does not prevent uninstall or force-quit itself.
            </span>
          </p>
        )}
        {showReconnectBrowser && (
          <p>
            <Link
              className="button secondary"
              to="/settings?tab=Connections"
              aria-label="Connect browser from Settings Connections"
              onClick={(e) => e.stopPropagation()}
            >
              Connect browser
            </Link>
            <span className="small muted" style={{ marginLeft: '0.75rem' }}>
              Browser disconnected — apps stay blocked. Reconnect to resume
              website blocking.
            </span>
          </p>
        )}
        {cameraBlocking && (
          <p className="camera-warn" role="alert">
            Camera isn&apos;t detecting you — timer is paused. Sit in front of
            the camera to resume.
          </p>
        )}
      </section>
      {detailsOpen && (
        <ActiveSessionDetailsModal
          sessionId={session.id}
          onClose={() => setDetailsOpen(false)}
        />
      )}
    </>
  );
}
