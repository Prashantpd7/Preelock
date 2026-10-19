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
import { countdown, formatDuration } from '../services/analytics';
import { api, desktop } from '../services/api';
import { Modal } from './SettingsControls';
import { BrowserSetup } from './BrowserSetup';
import type { FocusSetup, Session } from '../types';
function prettyApp(id: string, names?: Record<string, string>) {
  if (names?.[id]) return names[id];
  const tail = id.split('.').pop() || id;
  return tail.replace(/[-_]/g, ' ').trim() || id;
}
export function SessionCard({
  session,
  onEdit,
  appNames,
  setup,
  onSetupRefresh,
}: {
  session: Session;
  onEdit?: (session: Session) => void;
  appNames?: Record<string, string>;
  setup?: FocusSetup | null;
  onSetupRefresh?: () => Promise<FocusSetup | null>;
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
  const autoStarted = useRef(false);
  async function requestStart() {
    if (!desktop || allReady) {
      void execute({ type: 'start_session', id: session.id });
      return;
    }
    autoStarted.current = false;
    setCameraWaiting(false);
    setShowBrowserSetup(false);
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
  const browserOk =
    !!setup?.browser.connected && !setup?.browser.error;
  const browserReady = !needsBrowser || browserOk;
  const allReady = cameraReady && browserReady;
  const cameraState = cameraReady
    ? 'ready'
    : cameraBlocked
      ? 'blocked'
      : cameraWaiting
        ? 'waiting'
        : 'ask';
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
    : verification === 'manual_checkpoints'
      ? 'You confirm you are working when asked.'
      : verification === 'camera_and_activity'
        ? "Checks your apps and that you're at your desk."
        : 'Checks which app you are using.';
  const styleText = strictMode
    ? 'Strict — no pausing. Use Emergency exit to stop early.'
    : session.allow_pause
      ? 'Flexible — you can pause anytime.'
      : 'Pausing is turned off for this session.';
  const websiteRows: string[] = [];
  const allowedSites = strictMode ? (focusCfg?.domains ?? []) : [];
  const earningSites = !strictMode
    ? (focusCfg?.attention?.work_domains ?? [])
    : [];
  const blockedSites = !strictMode ? (focusCfg?.domains ?? []) : [];
  if (allowedSites.length)
    websiteRows.push(`Only these are allowed: ${allowedSites.join(', ')}`);
  if (earningSites.length)
    websiteRows.push(`These earn focus time: ${earningSites.join(', ')}`);
  if (blockedSites.length)
    websiteRows.push(`Blocked: ${blockedSites.join(', ')}`);
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
        </div>
        {session.status === 'ready' && (
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
                </div>
                {session.status === 'running' &&
                  session.focus.attention.verification ===
                    'manual_checkpoints' && (
                    <button
                      disabled={busy}
                      onClick={() =>
                        void execute({ type: 'confirm_work', id: session.id })
                      }
                    >
                      I'm working · count 1 minute
                    </button>
                  )}
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
          {report &&
            !['ready', 'preparing'].includes(session.status) &&
            session.focus?.activity_consent && (
              <details className="detail-block">
                <summary>App & website activity</summary>
                {live && report.enforcement && (
                  <p className="small">{report.enforcement}</p>
                )}
                {live && (
                  <p className="muted small">
                    Now using: {report.active_app || 'nothing yet'}
                  </p>
                )}
                <div className="report-metrics">
                  <span>On-task {formatDuration(report.approved_ms)}</span>
                  <span>
                    Distracting {formatDuration(report.distracting_ms)}
                  </span>
                  <span>Idle {formatDuration(report.idle_ms)}</span>
                  <span>
                    {report.interruptions} interruption
                    {report.interruptions === 1 ? '' : 's'}
                  </span>
                </div>
              </details>
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
                blocked, use Emergency unblock in the browser extension.
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
                {cameraState === 'ready' ? (
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
                    <p className="muted small">
                      Your video never leaves this Mac.
                    </p>
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
                    setup={setup ?? null}
                    refresh={async () => {
                      await onSetupRefresh?.();
                    }}
                    browserId={session.focus?.browser_app ?? ''}
                  />
                ) : (
                  <>
                    <p>
                      This session needs the browser extension paired to
                      enforce its website rules.
                    </p>
                    <button
                      type="button"
                      onClick={() => setShowBrowserSetup(true)}
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
