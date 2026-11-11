import { useCallback, useEffect, useRef, useState } from 'react';
import { api, desktop } from '../services/api';
import { missingPrerequisites } from '../services/focusSetup';
import {
  autoStartedLiveSessions,
  minutesUntilStart,
  upcomingStarts,
} from '../services/preSessionWarning';
import { useAppStore } from '../state/store';
import type { FocusConfig, FocusSetup } from '../types';
import { BrowserSetup } from './BrowserSetup';
import { Modal } from './SettingsControls';

type WarningTarget =
  | {
      mode: 'upcoming';
      key: string;
      title: string;
      startMs: number;
      focus: FocusConfig;
    }
  | {
      mode: 'live';
      key: string;
      title: string;
      focus: FocusConfig;
      sessionId: string;
    };

/** Pre-session readiness warning, mounted app-wide in AppLayout.
 *
 * Two moments, one popup:
 * - 5 minutes before a scheduled or one-shot session auto-starts,
 *   the popup warns it is coming and shows what is still missing
 *   (browser not connected, camera access not granted), with the
 *   same connect / grant actions as the manual "Before you start"
 *   dialog. When nothing is missing it is just the plain notice.
 * - At start time, if the session started anyway and a
 *   prerequisite is still missing, the popup appears again for
 *   that session, so the user can fix it and the session can
 *   proceed (its lifecycle is unchanged: the engine keeps the
 *   session live, in grace, until the browser applies the policy).
 *
 * The popup never cancels, ends, or changes a session: dismissing
 * it only closes the dialog (one key per start / per session, so
 * a dismissed popup never reappears for the same start). The OS
 * notification is the same warning through the existing advance
 * warnings; this popup is the in-app fallback when notifications
 * are off, and it does not consult the notification preference. */
export function PreSessionWarning() {
  const data = useAppStore((s) => s.data);
  const [target, setTarget] = useState<WarningTarget | null>(null);
  const [targetSetup, setTargetSetup] = useState<FocusSetup | null>(null);
  const [setupLoading, setSetupLoading] = useState(false);
  const [showBrowserSetup, setShowBrowserSetup] = useState(false);
  const [allowingCamera, setAllowingCamera] = useState(false);
  const [cameraWaiting, setCameraWaiting] = useState(false);
  const [checking, setChecking] = useState(false);
  const shownPre = useRef(new Set<string>());
  const shownLive = useRef(new Set<string>());
  const openedWithMissing = useRef(false);
  // Live clock for the "starts in N minutes" line (Dashboard
  // pattern): the render must stay pure, so Date.now() lives here,
  // not in the render body (react-hooks/purity).
  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    const immediate = window.setTimeout(() => setNowMs(Date.now()), 0);
    const timer = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => {
      window.clearTimeout(immediate);
      window.clearInterval(timer);
    };
  }, []);
  // Mirrors `target` for the detection effect: listing `target` as a
  // dependency would make the effect's own open cancel its in-flight
  // setup fetch (the cleanup runs when the target changes).
  const targetRef = useRef<WarningTarget | null>(null);

  const closeTarget = useCallback(() => {
    targetRef.current = null;
    setTarget(null);
    setTargetSetup(null);
    setCameraWaiting(false);
    setShowBrowserSetup(false);
  }, []);

  const refreshTargetSetup = useCallback(async () => {
    if (!target) return;
    try {
      setTargetSetup(await api.focusSetup(target.focus.browser_app ?? ''));
    } catch {
      // Keep the last setup; the guide shows its own notice.
    }
  }, [target]);

  // A new target starts with a clean slate for the auto-close
  // tracking below.
  useEffect(() => {
    if (target) openedWithMissing.current = false;
  }, [target]);

  // Detect the next warning moment. Runs on every snapshot poll
  // (1s); the shown-key sets make each popup fire exactly once.
  // It is invoked through setTimeout, not called directly from the
  // effect: opening the popup is a state update, and setState
  // directly inside an effect is a cascading render
  // (react-hooks/set-state-in-effect) — a timer callback is the
  // same escape the close-at-start effect below uses.
  const detectWarning = useCallback(() => {
    if (targetRef.current || !desktop) return;
    const now = Date.now();
    const next = upcomingStarts(data, now)[0];
    if (next) {
      const key = `pre:${next.kind}:${next.id}:${next.startMs}`;
      if (!shownPre.current.has(key)) {
        shownPre.current.add(key);
        setSetupLoading(true);
        setTargetSetup(null);
        api
          .focusSetup(next.focus.browser_app ?? '')
          .then((fresh) => {
            if (targetRef.current?.key === key) setTargetSetup(fresh);
          })
          .catch(() => {})
          .finally(() => {
            if (targetRef.current?.key === key) setSetupLoading(false);
          });
        const upcomingTarget: WarningTarget = {
          mode: 'upcoming',
          key,
          title: next.title,
          startMs: next.startMs,
          focus: next.focus,
        };
        targetRef.current = upcomingTarget;
        setTarget(upcomingTarget);
        return;
      }
    }
    for (const session of autoStartedLiveSessions(data)) {
      const key = `live:${session.id}`;
      if (shownLive.current.has(key)) continue;
      // Marked before the fetch, so the setup is fetched once per
      // session even while it is still loading.
      shownLive.current.add(key);
      // A live session without a focus config cannot be checked
      // for prerequisites; skip it rather than guessing.
      const focus = session.focus;
      if (!focus) continue;
      setSetupLoading(true);
      api
        .focusSetup(focus.browser_app ?? '')
        .then((fresh) => {
          // Another popup opened meanwhile: leave its setup alone.
          if (targetRef.current) return;
          setTargetSetup(fresh);
          const missing = missingPrerequisites(focus, fresh);
          if (missing.browser || missing.camera) {
            const liveTarget: WarningTarget = {
              mode: 'live',
              key,
              title: session.title,
              focus,
              sessionId: session.id,
            };
            targetRef.current = liveTarget;
            setTarget(liveTarget);
          }
        })
        .catch(() => {
          shownLive.current.delete(key);
        })
        .finally(() => {
          if (!targetRef.current || targetRef.current.key === key) {
            setSetupLoading(false);
          }
        });
      break;
    }
  }, [data]);

  useEffect(() => {
    const id = window.setTimeout(detectWarning, 0);
    return () => window.clearTimeout(id);
  }, [data, detectWarning]);

  // Close a warning that is no longer relevant: upcoming starts once
  // their start time passes or their schedule/one-shot vanishes, and
  // live warnings once that session ends. The close is scheduled
  // rather than called directly: setState inside an effect body is a
  // cascading render (react-hooks/set-state-in-effect).
  useEffect(() => {
    if (!target) return;
    const now = Date.now();
    const staleUpcoming =
      target.mode === 'upcoming' &&
      (now >= target.startMs ||
        !upcomingStarts(data, now).some(
          (item) =>
            `pre:${item.kind}:${item.id}:${item.startMs}` === target.key,
        ));
    const staleLive =
      target.mode === 'live' &&
      !data.sessions.some(
        (session) =>
          session.id === target.sessionId &&
          (session.status === 'preparing' ||
            session.status === 'running' ||
            session.status === 'paused'),
      );
    if (staleUpcoming || staleLive) {
      const id = window.setTimeout(closeTarget, 0);
      return () => window.clearTimeout(id);
    }
  }, [data, target, closeTarget]);

  // Auto-close once fixed: if the popup opened while something was
  // missing and the fresh per-session setup now shows everything
  // ready, the user fixed it — close on its own.
  useEffect(() => {
    if (!target || !targetSetup) return;
    const missing = missingPrerequisites(target.focus, targetSetup);
    if (missing.browser || missing.camera) {
      openedWithMissing.current = true;
    } else if (openedWithMissing.current) {
      closeTarget();
    }
  }, [target, targetSetup, closeTarget]);

  async function allowCamera() {
    setAllowingCamera(true);
    try {
      await api.requestCamera();
    } finally {
      setAllowingCamera(false);
    }
    setCameraWaiting(true);
  }

  async function recheck() {
    setChecking(true);
    try {
      await refreshTargetSetup();
    } finally {
      setChecking(false);
    }
  }

  if (!target) return null;
  // Before the per-session setup loads, nothing is reported
  // missing: the popup shows the plain notice plus "Checking…"
  // until the check lands (a local call, milliseconds).
  const missing = targetSetup
    ? missingPrerequisites(target.focus, targetSetup)
    : { browser: false, camera: false };
  const minutes =
    target.mode === 'upcoming' ? minutesUntilStart(target.startMs, nowMs) : 0;
  const minuteWord = minutes === 1 ? 'minute' : 'minutes';
  const cameraBlocked =
    targetSetup?.camera_permission === 'denied' ||
    targetSetup?.camera_permission === 'restricted';
  const title = missing.browser
    ? 'Connect browser to start this session'
    : missing.camera
      ? 'Allow camera to start this session'
      : 'Session starting soon';
  return (
    <Modal title={title} onClose={closeTarget}>
      <div className="dialog-stack">
        {target.mode === 'upcoming' ? (
          <p>
            &ldquo;{target.title}&rdquo; starts in {minutes} {minuteWord}.
          </p>
        ) : (
          <p>
            &ldquo;{target.title}&rdquo; started, but{' '}
            {missing.browser && missing.camera
              ? 'the browser is not connected and camera access is not granted'
              : missing.browser
                ? 'the browser is not connected, so its website rules are not being enforced'
                : 'camera access is not granted, so camera checks are not running'}
            .
          </p>
        )}
        {setupLoading && !targetSetup && (
          <p className="muted small" role="status">
            Checking browser and camera…
          </p>
        )}
        {missing.browser && (
          <section className="ready-check">
            <h3>Browser extension</h3>
            {showBrowserSetup ? (
              <BrowserSetup
                setup={targetSetup}
                refresh={refreshTargetSetup}
                browserId={target.focus.browser_app ?? ''}
              />
            ) : (
              <>
                <p>
                  This session needs the browser extension paired to enforce its
                  website rules.
                </p>
                <button type="button" onClick={() => setShowBrowserSetup(true)}>
                  Connect browser
                </button>
              </>
            )}
          </section>
        )}
        {missing.camera && (
          <section className="ready-check">
            <h3>Camera</h3>
            {targetSetup?.platform === 'windows' ? (
              <p>
                Camera checks are not available on Windows. Edit this session
                and choose Apps only.
              </p>
            ) : cameraBlocked ? (
              <>
                <p>
                  Camera access is blocked for Preelock, so this session
                  can&apos;t start its camera check.
                </p>
                <p className="muted small">
                  Turn on Preelock in System Settings → Privacy & Security →
                  Camera, then check again.
                </p>
                <div className="row controls">
                  <button
                    type="button"
                    className="secondary"
                    onClick={() => {
                      closeTarget();
                      void api.openSetup('camera_settings');
                    }}
                  >
                    Open camera settings
                  </button>
                  <button
                    type="button"
                    onClick={() => void recheck()}
                    disabled={checking}
                  >
                    {checking ? 'Checking…' : 'Check again'}
                  </button>
                </div>
              </>
            ) : cameraWaiting ? (
              <>
                <p>Answer the camera prompt from macOS, then check below.</p>
                <button
                  type="button"
                  onClick={() => void recheck()}
                  disabled={checking}
                >
                  {checking ? 'Checking…' : "I've allowed it"}
                </button>
              </>
            ) : (
              <>
                <p>
                  This session checks that you&apos;re at your desk using your
                  camera.
                </p>
                <p className="muted small">Video stays on this device.</p>
                <div className="row controls">
                  <button
                    type="button"
                    className="secondary"
                    onClick={closeTarget}
                  >
                    Not now
                  </button>
                  <button
                    type="button"
                    onClick={() => void allowCamera()}
                    disabled={allowingCamera}
                  >
                    {allowingCamera ? 'Asking…' : 'Allow camera'}
                  </button>
                </div>
              </>
            )}
          </section>
        )}
        {!missing.browser && !missing.camera && (
          <div className="row controls" style={{ justifyContent: 'flex-end' }}>
            <button type="button" onClick={closeTarget}>
              Got it
            </button>
          </div>
        )}
      </div>
    </Modal>
  );
}
