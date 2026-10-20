import { useEffect, useRef, useState } from 'react';
import {
  MoreVertical,
  Pause,
  Pencil,
  Play,
  Square,
  Timer,
  Trash2,
} from 'lucide-react';
import { useAppStore } from '../state/store';
import { countdown } from '../services/analytics';
import { api, desktop } from '../services/api';
import { Modal } from './SettingsControls';
import { BrowserSetup } from './BrowserSetup';
import { sameBrowser, sessionBrowsers } from '../types';
import type { FocusSetup, Session } from '../types';
function prettyApp(id: string, names?: Record<string, string>) {
  if (names?.[id]) return names[id];
  const tail = id.split('.').pop() || id;
  return tail.replace(/[-_]/g, ' ').trim() || id;
}
function formatStartedAt(ms: number) {
  return new Date(ms).toLocaleString(undefined, {
    day: 'numeric',
    month: 'short',
    year: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
}
export function SessionCard({
  session,
  onEdit,
  appNames,
  setup,
  onSetupRefresh,
  showStartedAt,
  hideStart,
}: {
  session: Session;
  onEdit?: (session: Session) => void;
  appNames?: Record<string, string>;
  setup?: FocusSetup | null;
  onSetupRefresh?: () => Promise<FocusSetup | null>;
  showStartedAt?: boolean;
  hideStart?: boolean;
}) {
  const execute = useAppStore((s) => s.execute);
  const busy = useAppStore((s) => s.busy);
  const [open, setOpen] = useState(false);
  const [menu, setMenu] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [recovery, setRecovery] = useState(false);
  const [confirmation, setConfirmation] = useState('');
  const [readyOpen, setReadyOpen] = useState(false);
  const [cameraWaiting, setCameraWaiting] = useState(false);
  const [allowingCamera, setAllowingCamera] = useState(false);
  const [checking, setChecking] = useState(false);
  const [showBrowserSetup, setShowBrowserSetup] = useState(false);
  // Which browser the connect guide targets: the first unpaired one.
  const [guideBrowser, setGuideBrowser] = useState('');
  // Connection status for this session's own browser, fetched when the
  // pre-start popup opens. Falls back to the shared setup until loaded.
  const [sessionSetup, setSessionSetup] = useState<FocusSetup | null>(null);
  // Connection status for the browser the embedded connect guide targets.
  // Kept separate from sessionSetup: the guide's browser can differ from
  // the session's dropdown browser, and reusing a status filtered to an
  // already-connected browser would render the guide as nothing.
  const [guideSetup, setGuideSetup] = useState<FocusSetup | null>(null);
  const autoStarted = useRef(false);
  const finished = ['completed', 'interrupted', 'ended'].includes(
    session.status,
  );
  async function requestStart() {
    if (finished) {
      // Reuse the card: reset it to a fresh run first.
      const ok = await execute({ type: 'restart_session', id: session.id });
      if (!ok) return;
    }
    if (!desktop || allReady) {
      void execute({ type: 'start_session', id: session.id });
      return;
    }
    autoStarted.current = false;
    setCameraWaiting(false);
    setShowBrowserSetup(false);
    setGuideBrowser('');
    setGuideSetup(null);
    setReadyOpen(true);
  }
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
      await onSetupRefresh?.();
    } finally {
      setChecking(false);
    }
  }
  const live = ['running', 'paused', 'preparing'].includes(session.status);
  const committed = session.focus && session.focus.mode !== 'standard';
  const report = session.report;
  const attention = session.attention;
  const stateNames: Record<string, string> = {
    IDLE: 'Ready',
    PREPARING: 'Connecting',
    FOCUSED: 'Focusing',
    DISTRACTED: 'Off task',
    ABSENT: 'Away',
    UNKNOWN: 'Checking focus',
    RECOVERING: 'Checking connection',
    COMPLETED: 'Completed',
    EMERGENCY_EXIT: 'Stopped',
  };
  const closeMenu = () => {
    setMenu(false);
    setConfirmDelete(false);
  };
  const focusCfg = session.focus;
  const needsCamera =
    focusCfg?.camera === true ||
    focusCfg?.attention?.verification === 'camera_and_activity';
  const needsBrowser =
    !!focusCfg &&
    (!focusCfg.general_internet ||
      (focusCfg.domains?.length ?? 0) > 0 ||
      (focusCfg.mode !== 'standard' && !!focusCfg.browser_app) ||
      (focusCfg.attention?.work_domains?.length ?? 0) > 0);
  const cameraPermission = setup?.camera_permission;
  const cameraBlocked =
    cameraPermission === 'denied' || cameraPermission === 'restricted';
  const cameraReady = !needsCamera || cameraPermission === 'granted';
  const cameraStopped = (report?.events ?? []).some(
    (e) => e.kind === 'camera_disabled',
  );
  // The paired extension must report that it lives in this session's
  // browser — a connection from any other browser does not count.
  // Prefers the per-session status fetched for the popup; falls back to
  // the shared setup until it loads.
  const browserSetup = sessionSetup ?? setup;
  // Strict sessions enforce the website rules in every selected browser,
  // so all of them need the extension paired.
  const sessionBrowserIds = focusCfg ? sessionBrowsers(focusCfg) : [];
  const connectedIds = browserSetup?.connected_browsers ?? [];
  const unpairedBrowsers = sessionBrowserIds.filter(
    (id) => !connectedIds.some((c) => sameBrowser(c, id)),
  );
  const browserOk =
    !!browserSetup?.browser.connected &&
    !browserSetup?.browser.error &&
    (!focusCfg?.browser_app ||
      (!!browserSetup?.browser.client_browser &&
        sameBrowser(
          focusCfg.browser_app,
          browserSetup.browser.client_browser,
        ))) &&
    unpairedBrowsers.length === 0;
  const browserReady = !needsBrowser || browserOk;
  const allReady = cameraReady && browserReady;
  const cameraState = cameraReady
    ? 'ready'
    : cameraBlocked
      ? 'blocked'
      : cameraWaiting
        ? 'waiting'
        : 'ask';
  // Fetch this session's own browser connection when the pre-start popup
  // opens — it can differ from the most recently used browser elsewhere.
  useEffect(() => {
    if (!readyOpen || !desktop) return;
    let active = true;
    api
      .focusSetup(session.focus?.browser_app ?? '')
      .then((v) => {
        if (active) setSessionSetup(v);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [readyOpen, session.focus?.browser_app]);
  useEffect(() => {
    if (!readyOpen || !allReady || autoStarted.current) return;
    autoStarted.current = true;
    setReadyOpen(false);
    void execute({ type: 'start_session', id: session.id });
  }, [readyOpen, allReady, execute, session.id]);
  const strictMode = focusCfg?.mode === 'strict';
  const verification = focusCfg?.attention?.verification;
  const workAppIds = Array.from(
    new Set([
      ...(focusCfg?.attention?.work_apps ?? []),
      ...(focusCfg?.apps ?? []),
    ]),
  );
  const workAppNames = workAppIds.map((id) => prettyApp(id, appNames));
  const verificationText = !focusCfg?.attention?.enabled
    ? 'Timer only — no focus checks.'
    : verification === 'camera_and_activity'
      ? "Checks your apps and that you're at your desk."
      : 'Checks which app you are using.';
  const styleText = strictMode
    ? 'Strict — no pausing. Use Emergency exit to stop early.'
    : session.allow_pause
      ? 'Flexible — you can pause anytime.'
      : 'Pausing is turned off for this session.';
  const websiteRows: string[] = [];
  const blocking = focusCfg?.website_blocking ?? 'auto';
  const effectiveBlocking =
    blocking === 'auto' ? (strictMode ? 'allowlist' : 'blocklist') : blocking;
  const domains = focusCfg?.domains ?? [];
  const earningSites = focusCfg?.attention?.work_domains ?? [];
  if (earningSites.length)
    websiteRows.push(`These earn focus time: ${earningSites.join(', ')}`);
  if (effectiveBlocking === 'allowlist' && domains.length)
    websiteRows.push(`Only these are open: ${domains.join(', ')}`);
  if (effectiveBlocking === 'blocklist' && domains.length)
    websiteRows.push(`Blocked: ${domains.join(', ')}`);
  const outcomeText =
    session.status === 'completed'
      ? 'Completed the full session.'
      : session.status === 'ended'
        ? 'Ended early by you.'
        : session.status === 'interrupted'
          ? 'Stopped early — blocking was turned off.'
          : null;
  return (
    <article className={`panel session-card ${live ? 'active-session' : ''}`}>
      <div
        className="row session-head"
        role="button"
        tabIndex={0}
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        onMouseDown={(e) => e.preventDefault()}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            setOpen((o) => !o);
          }
        }}
      >
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
          {session.force_quit && (
            <p className="force-quit-flag" role="note">
              Force quit during session
            </p>
          )}
        </div>
        {live && (
          <span
            className={`live-indicator${session.status === 'paused' ? ' paused' : ''}`}
            role="status"
            aria-label={
              session.status === 'paused' ? 'Session paused' : 'Session active'
            }
          >
            <span className="live-dot" aria-hidden="true" />
            {session.status === 'paused'
              ? 'Paused'
              : countdown(session.remaining_ms)}
          </span>
        )}
        {!hideStart &&
          ['ready', 'completed', 'interrupted', 'ended'].includes(
            session.status,
          ) && (
            <button
              disabled={busy || !desktop}
              onClick={(e) => {
                e.stopPropagation();
                void requestStart();
              }}
            >
              <Play size={15} />
              Start session
            </button>
          )}
        {session.status === 'running' && session.allow_pause && !committed && (
          <button
            className="secondary"
            disabled={busy}
            onClick={(e) => {
              e.stopPropagation();
              void execute({ type: 'pause_session', id: session.id });
            }}
          >
            <Pause size={15} />
            Pause
          </button>
        )}
        {session.status === 'paused' && (
          <button
            disabled={busy}
            onClick={(e) => {
              e.stopPropagation();
              void execute({ type: 'resume_session', id: session.id });
            }}
          >
            <Play size={15} />
            Resume
          </button>
        )}
        {showStartedAt && session.started_at != null && (
          <span
            className="started-at"
            title={new Date(session.started_at).toLocaleString()}
          >
            {formatStartedAt(session.started_at)}
          </span>
        )}
        {!live && (
          <div className="menu-wrap">
            <button
              className="icon-button"
              aria-label="Session options"
              aria-haspopup="menu"
              onClick={(e) => {
                e.stopPropagation();
                if (menu) closeMenu();
                else setMenu(true);
              }}
            >
              <MoreVertical size={18} />
            </button>
            {menu && (
              <>
                <div
                  className="menu-backdrop"
                  onClick={(e) => {
                    e.stopPropagation();
                    closeMenu();
                  }}
                />
                <div className="menu" role="menu">
                  {session.status === 'ready' && onEdit && (
                    <button
                      type="button"
                      role="menuitem"
                      onClick={(e) => {
                        e.stopPropagation();
                        closeMenu();
                        onEdit?.(session);
                      }}
                    >
                      <Pencil size={15} />
                      Edit the session
                    </button>
                  )}
                  <button
                    type="button"
                    role="menuitem"
                    className="danger"
                    onClick={(e) => {
                      e.stopPropagation();
                      if (!confirmDelete) {
                        setConfirmDelete(true);
                        return;
                      }
                      closeMenu();
                      void execute({ type: 'delete_session', id: session.id });
                    }}
                  >
                    <Trash2 size={15} />
                    {confirmDelete ? 'Confirm delete?' : 'Delete session'}
                  </button>
                </div>
              </>
            )}
          </div>
        )}
      </div>
      {open && (
        <div className="session-details">
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
          <section className="detail-block" aria-label="About this session">
            <h4>About this session</h4>
            <dl className="detail-list">
              <div>
                <dt>Length</dt>
                <dd>{session.duration_minutes} minutes</dd>
              </div>
              <div>
                <dt>Style</dt>
                <dd>{styleText}</dd>
              </div>
              <div>
                <dt>Focus checks</dt>
                <dd>{verificationText}</dd>
              </div>
              {workAppNames.length > 0 && (
                <div>
                  <dt>Work apps</dt>
                  <dd>{workAppNames.join(', ')}</dd>
                </div>
              )}
              {websiteRows.length > 0 && (
                <div>
                  <dt>Websites</dt>
                  <dd>{websiteRows.join(' · ')}</dd>
                </div>
              )}
              <div>
                <dt>Camera</dt>
                <dd>
                  {focusCfg?.camera
                    ? "On — checks you're at your desk."
                    : 'Off.'}
                </dd>
              </div>
            </dl>
          </section>
          {session.focus?.attention?.enabled &&
            attention &&
            !['ready', 'preparing'].includes(session.status) && (
              <section
                className="detail-block"
                aria-label={live ? 'Time so far' : 'Final result'}
              >
                <h4>{live ? 'Time so far' : 'Final result'}</h4>
                {live ? (
                  <p className="small">
                    Right now:{' '}
                    <strong>
                      {session.status === 'paused'
                        ? 'Paused'
                        : stateNames[attention.state] || attention.state}
                    </strong>
                  </p>
                ) : (
                  outcomeText && <p className="small">{outcomeText}</p>
                )}
                <div className="report-metrics">
                  <span>Focused {countdown(session.focused_ms)}</span>
                  <span>Distracted {countdown(attention.distracted_ms)}</span>
                  <span>Away {countdown(attention.absent_ms)}</span>
                  <span>
                    Total{' '}
                    {countdown(
                      session.focused_ms +
                        attention.distracted_ms +
                        attention.absent_ms,
                    )}
                  </span>
                </div>
              </section>
            )}
          {!session.focus?.attention?.enabled && (
            <p className="muted small">
              Older timer session · no focus checks.
            </p>
          )}
          {session.status === 'preparing' && (
            <p role="status" className="notice">
              Connecting to your browser… Open its VYSTRIQ extension and click
              Reconnect.
            </p>
          )}
          {live && session.focus?.camera && (
            <p className="small" role="status">
              Camera:{' '}
              {report?.camera_status?.replaceAll('_', ' ') || 'starting'}
            </p>
          )}
          {live && (
            <div className="row controls">
              {live && !committed && (
                <button
                  className="secondary"
                  disabled={busy}
                  onClick={() =>
                    void execute({ type: 'end_session', id: session.id })
                  }
                >
                  <Square size={14} />
                  End session
                </button>
              )}
              {live && (
                <button
                  className="secondary"
                  onClick={() => setRecovery(!recovery)}
                >
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
              {!session.focus?.camera &&
                live &&
                cameraStopped &&
                !cameraBlocked && (
                  <button
                    className="secondary"
                    disabled={busy}
                    onClick={() =>
                      void execute({ type: 'enable_camera', id: session.id })
                    }
                  >
                    Start camera
                  </button>
                )}
            </div>
          )}
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
                Stop this session and release its blocks. If websites stay
                blocked, disconnect Vystriq in the browser extension.
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
        </div>
      )}
      {readyOpen && (
        <Modal title="Before you start" onClose={() => setReadyOpen(false)}>
          <div className="dialog-stack">
            {needsCamera && (
              <section className="ready-check">
                <h3>Camera</h3>
                {setup?.platform === 'windows' ? (
                  <p>
                    Camera checks are not available on Windows. Edit this
                    session and choose Apps only.
                  </p>
                ) : cameraState === 'ready' ? (
                  <p className="ready-box">Ready</p>
                ) : cameraState === 'waiting' ? (
                  <>
                    <p>
                      Answer the camera prompt from macOS, then check below.
                    </p>
                    <button
                      type="button"
                      onClick={() => void recheck()}
                      disabled={checking}
                    >
                      {checking ? 'Checking…' : "I've allowed it"}
                    </button>
                  </>
                ) : cameraState === 'blocked' ? (
                  <>
                    <p>
                      Camera access is blocked for VYSTRIQ, so this session
                      can't start its camera check.
                    </p>
                    <p className="muted small">
                      Turn on VYSTRIQ in System Settings → Privacy & Security →
                      Camera, then check again.
                    </p>
                    <div className="row controls">
                      <button
                        type="button"
                        className="secondary"
                        onClick={() => {
                          setReadyOpen(false);
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
                ) : (
                  <>
                    <p>
                      This session checks that you're at your desk using your
                      camera.
                    </p>
                    <p className="muted small">Video stays on this device.</p>
                    <div className="row controls">
                      <button
                        type="button"
                        className="secondary"
                        onClick={() => setReadyOpen(false)}
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
            {needsBrowser && (
              <section className="ready-check">
                <h3>Browser extension</h3>
                {browserReady ? (
                  <p className="ready-box">Ready</p>
                ) : showBrowserSetup ? (
                  <BrowserSetup
                    setup={guideSetup}
                    refresh={async () => {
                      const target =
                        guideBrowser || session.focus?.browser_app || '';
                      setGuideSetup(await api.focusSetup(target));
                      // Keep the popup's own readiness check live so it
                      // flips to Ready (and auto-starts) once paired.
                      setSessionSetup(
                        await api.focusSetup(session.focus?.browser_app ?? ''),
                      );
                      await onSetupRefresh?.();
                    }}
                    browserId={guideBrowser || session.focus?.browser_app || ''}
                  />
                ) : (
                  <>
                    <p>
                      This session needs the browser extension paired to enforce
                      its website rules.
                    </p>
                    <button
                      type="button"
                      onClick={() => {
                        const target =
                          unpairedBrowsers[0] ??
                          session.focus?.browser_app ??
                          '';
                        setGuideBrowser(target);
                        setGuideSetup(null);
                        setShowBrowserSetup(true);
                        // Fetch this browser's status now: the popup's
                        // sessionSetup is filtered to the session's dropdown
                        // browser and may already be connected, which would
                        // render the guide as nothing.
                        api
                          .focusSetup(target)
                          .then(setGuideSetup)
                          .catch(() => {});
                      }}
                    >
                      Connect browser extension
                    </button>
                  </>
                )}
              </section>
            )}
          </div>
        </Modal>
      )}
    </article>
  );
}
