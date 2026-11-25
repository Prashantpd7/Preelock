import { useEffect, useState } from 'react';
import { Pause, Play, Square } from 'lucide-react';
import { useAppStore } from '../state/store';
import { countdown } from '../services/analytics';
import {
  hasYouTubeRules,
  isLiveSessionStatus,
  liveRemainingMs,
  liveSession,
  pomodoroBreak,
} from '../services/activeSession';
import { useFocusSetup } from '../hooks/useFocusSetup';
import { ActiveSessionDetailsModal } from './ActiveSessionDetailsModal';
import { BrowserSetup } from './BrowserSetup';
import { Modal } from './SettingsControls';

/** Global active-session card, mounted once in AppLayout so it
 *  appears at the top of every main page. This is the Dashboard's
 *  banner moved here unchanged — same content, same live data from
 *  the shared store (which refreshes from the backend every
 *  second), plus click-to-open details. The whole card opens the
 *  details popup. No "Connect browser" option is shown once a
 *  session has started — the DNS proxy blocks websites, so a
 *  disconnected extension must never nag the user mid-session. */
export function ActiveSessionBanner() {
  const { sessions, browser } = useAppStore((s) => s.data);
  const execute = useAppStore((s) => s.execute);
  const busy = useAppStore((s) => s.busy);
  const startingSession = useAppStore((s) => s.startingSession);
  const setStartingSession = useAppStore((s) => s.setStartingSession);
  const session = liveSession(sessions);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [ytConnectModalOpen, setYtConnectModalOpen] = useState(false);
  const { setup, refresh } = useFocusSetup(session?.focus?.browser_app);
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
  if (ytConnectModalOpen && (!session || browser?.connected)) {
    setYtConnectModalOpen(false);
  }
  // The clock ticks from an effect, never from render (purity
  // rule): an immediate timeout syncs it the moment grace starts
  // (a mount-time initializer would go stale if the banner mounted
  // long before grace began), and the interval keeps it ticking.
  const [nowMs, setNowMs] = useState(() => Date.now());
  // Tick on the local clock for as long as a session is live, not only
  // during grace: the main countdown is derived from the session's
  // deadline below, so it keeps moving even when snapshots stop arriving.
  const sessionId = session?.id ?? null;
  const deadlineMs = session?.deadline_ms ?? null;
  useEffect(() => {
    if (!sessionId) return;
    const immediate = window.setTimeout(() => setNowMs(Date.now()), 0);
    const timer = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => {
      window.clearTimeout(immediate);
      window.clearInterval(timer);
    };
  }, [sessionId, deadlineMs]);
  const graceRemainingSecs =
    session?.status === 'running' && graceUntilMs
      ? Math.max(0, Math.ceil((graceUntilMs - nowMs) / 1000))
      : 0;
  const inGrace = graceRemainingSecs > 0;
  // Once the backend reports the real live session, the optimistic
  // "starting" entry is no longer needed — clear it so it can never
  // linger beside the real banner.
  if (session && startingSession) setStartingSession(null);
  if (!session) {
    // No live session yet, but the user just clicked Start: show the
    // banner instantly with the session they configured, instead of
    // leaving the page blank for 2–5s while the backend starts the
    // DNS proxy. The real session replaces this on the next snapshot.
    if (!startingSession) return null;
    return (
      <section
        className="panel active-session-banner"
        aria-live="polite"
        aria-label={`Starting session: ${startingSession.title}.`}
      >
        <p className="active-session-kicker">
          <span className="live-dot" aria-hidden="true" />
          Active session · {startingSession.title} · starting…
        </p>
        <div className="active-session-main">
          <div>
            <p className="active-session-timer">
              {countdown(startingSession.minutes * 60 * 1000)}
            </p>
            <p className="small muted">
              Starting your session — blocking begins in a moment.
            </p>
          </div>
        </div>
      </section>
    );
  }
  // Pomodoro break, if one is on: the kicker suffix stays, and the
  // break timer on the right replaces the old sentence — no long
  // text on this card anymore.
  const breakInfo = pomodoroBreak(session);
  // Pause/Stop follow the SessionCard rules exactly: Pause only for a
  // running flexible session that allows it, Stop only when early
  // ending is allowed, and neither for a strict or frozen session.
  // (Locks are enforced by the backend: pausing or stopping a locked
  // session without proofs fails there, exactly as the SessionCard
  // head buttons behave. The details popup stays read-only.)
  const committed = session.focus?.mode !== 'standard';
  const frozen = !!session.focus?.frozen?.enabled;
  const canPause =
    session.status === 'running' &&
    session.allow_pause &&
    !committed &&
    !frozen;
  const canResume = session.status === 'paused';
  const canEnd = isLiveSessionStatus(session.status) && !committed && !frozen;
  // YouTube blocking option was selected during session creation
  const usesYouTubeBlocking = hasYouTubeRules(session.focus);
  // Show "Connect browser to use YouTube" only when YouTube blocking
  // was selected and the browser extension is currently disconnected.
  const showYtReconnect =
    isLiveSessionStatus(session.status) &&
    !inGrace &&
    usesYouTubeBlocking &&
    !browser?.connected;
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
          {breakInfo && ' · on break'}
        </p>
        <div className="active-session-main">
          <div>
            <p className="active-session-timer">
              {inGrace
                ? `Starts in ${graceRemainingSecs}s`
                : countdown(liveRemainingMs(session, nowMs))}
            </p>
            {!breakInfo && (
              <p className="small muted">
                {inGrace
                  ? 'Blocking begins when the countdown ends. Finish up — nothing is blocked yet.'
                  : session.status === 'preparing'
                    ? 'Waiting for the browser extension to connect…'
                    : 'Distracting apps and websites stay blocked until this session ends.'}
              </p>
            )}
          </div>
          {breakInfo && (
            <div className="break-timer" role="timer" aria-label="Break timer">
              <p className="break-timer-label">BREAK TIMER</p>
              <p className="break-timer-time">
                <svg
                  className="break-timer-clock"
                  viewBox="0 0 24 24"
                  width="26"
                  height="26"
                  aria-hidden="true"
                >
                  <circle
                    className="break-timer-clock-face"
                    cx="12"
                    cy="12"
                    r="10"
                  />
                  <line
                    className="break-timer-clock-tick"
                    x1="12"
                    y1="2.8"
                    x2="12"
                    y2="4.6"
                  />
                  <line
                    className="break-timer-clock-tick"
                    x1="21.2"
                    y1="12"
                    x2="19.4"
                    y2="12"
                  />
                  <line
                    className="break-timer-clock-tick"
                    x1="12"
                    y1="21.2"
                    x2="12"
                    y2="19.4"
                  />
                  <line
                    className="break-timer-clock-tick"
                    x1="2.8"
                    y1="12"
                    x2="4.6"
                    y2="12"
                  />
                  <line
                    className="break-timer-clock-hand-short"
                    x1="12"
                    y1="12"
                    x2="15.5"
                    y2="12"
                  />
                  <line
                    className="break-timer-clock-hand"
                    x1="12"
                    y1="12"
                    x2="12"
                    y2="5"
                  />
                  <circle
                    className="break-timer-clock-pin"
                    cx="12"
                    cy="12"
                    r="1.1"
                  />
                </svg>
                {countdown(breakInfo.remainingMs)}
              </p>
            </div>
          )}
          <div className="active-session-right">
            {showYtReconnect && (
              <button
                type="button"
                className="button secondary active-session-yt-connect-btn"
                onClick={(e) => {
                  e.stopPropagation();
                  setYtConnectModalOpen(true);
                }}
              >
                Connect browser to use YouTube
              </button>
            )}
            {(canPause || canResume || canEnd) && (
              <div className="row controls active-session-controls">
                {canPause && (
                  <button
                    type="button"
                    className="secondary"
                    disabled={busy}
                    onClick={(e) => {
                      e.stopPropagation();
                      void execute({
                        type: 'pause_session',
                        id: session.id,
                      });
                    }}
                  >
                    <Pause size={15} />
                    Pause
                  </button>
                )}
                {canResume && (
                  <button
                    type="button"
                    disabled={busy}
                    onClick={(e) => {
                      e.stopPropagation();
                      void execute({
                        type: 'resume_session',
                        id: session.id,
                      });
                    }}
                  >
                    <Play size={15} />
                    Resume
                  </button>
                )}
                {canEnd && (
                  <button
                    type="button"
                    className="secondary"
                    disabled={busy}
                    onClick={(e) => {
                      e.stopPropagation();
                      void execute({ type: 'end_session', id: session.id });
                    }}
                  >
                    <Square size={14} />
                    Stop
                  </button>
                )}
              </div>
            )}
          </div>
        </div>
        {cameraBlocking && (
          <p className="camera-warn" role="alert">
            Camera isn&apos;t detecting you — timer is paused. Sit in front of
            the camera to resume.
          </p>
        )}
      </section>
      {ytConnectModalOpen && (
        <Modal
          title="Connect browser to use YouTube"
          onClose={() => setYtConnectModalOpen(false)}
        >
          <div className="dialog-stack">
            <BrowserSetup
              setup={setup}
              refresh={refresh}
              browserId={session.focus?.browser_app}
              singleBrowserId={session.focus?.browser_app}
              onBack={() => setYtConnectModalOpen(false)}
            />
          </div>
        </Modal>
      )}
      {detailsOpen && (
        <ActiveSessionDetailsModal
          sessionId={session.id}
          onClose={() => setDetailsOpen(false)}
        />
      )}
    </>
  );
}
