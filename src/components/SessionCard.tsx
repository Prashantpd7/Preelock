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
import { isSafariId, sessionNeedsExtension } from '../services/browserSupport';
import { sessionTypeLabel, youtubePolicy } from '../services/activeSession';
import { Modal } from './SettingsControls';
import { BrowserSetup } from './BrowserSetup';
import { SessionReview } from './SessionReview';
import {
  frozenLabel,
  lockDelayStatus,
  lockHasGate,
  lockLabel,
  pomodoroLabel,
  sameBrowser,
  sessionBrowsers,
  tamperNote,
} from '../types';
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
  const [readyOpen, setReadyOpen] = useState(false);
  const [cameraWaiting, setCameraWaiting] = useState(false);
  const [allowingCamera, setAllowingCamera] = useState(false);
  const [checking, setChecking] = useState(false);
  const [showBrowserSetup, setShowBrowserSetup] = useState(false);
  // Step 10 unlock proofs, entered in the details lock form.
  const [unlockPassword, setUnlockPassword] = useState('');
  const [unlockChallenge, setUnlockChallenge] = useState('');
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
  const tamper = tamperNote(session);
  const lockText = lockLabel(session.lock);
  // Step 12 Frozen Turkey: badge text, and whether this session
  // can ever be paused (it cannot — the backend rejects it).
  const frozen = !!session.focus?.frozen?.enabled;
  const frozenText = frozenLabel(session.focus);
  const gated = live && lockHasGate(session.lock);
  const lockDelay = lockDelayStatus(session.lock_state);
  const lockDelayPending = gated && lockDelay.kind === 'pending';
  const lockDelayDone = gated && lockDelay.kind === 'done';
  const lockDelayFor = session.lock_state?.delay_for ?? null;
  const proofs = {
    password: unlockPassword || undefined,
    challenge_answer: unlockChallenge || undefined,
  };
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
      (focusCfg.attention?.work_domains?.length ?? 0) > 0 ||
      (focusCfg.youtube_blocked_channels?.length ?? 0) > 0 ||
      (focusCfg.youtube_allowed_channels?.length ?? 0) > 0 ||
      !!focusCfg.youtube_study_mode ||
      // Step 12: a frozen session with a picked browser needs
      // that browser connected (backend preflight requires it);
      // with no picked browser, no browser check applies.
      (!!focusCfg.frozen?.enabled && !!focusCfg.browser_app));
  const cameraPermission = setup?.camera_permission;
  const cameraBlocked =
    cameraPermission === 'denied' || cameraPermission === 'restricted';
  const cameraReady =
    !needsCamera ||
    setup?.platform === 'windows' ||
    cameraPermission === 'granted';
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
  // Review shown at the top of the pre-start popup: the exact
  // config this saved session will start with. Safari can never
  // enforce website rules; the review warns about it (never on
  // Windows — Safari is macOS-only — the review gates that).
  const safariWarning =
    !!focusCfg &&
    sessionNeedsExtension(focusCfg) &&
    (isSafariId(focusCfg.browser_app) ||
      (focusCfg.apps ?? []).some((id) => isSafariId(id)));
  const reviewBrowserName =
    (browserSetup?.browsers ?? []).find((b) =>
      sameBrowser(b.id, focusCfg?.browser_app ?? ''),
    )?.name ?? '';
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
    ? 'Strict — no pausing or ending early.'
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
  // YouTube restrictions, in the same plain words as the live
  // session popup: blocked channels, or the study-mode allowlist.
  // No row at all when nothing was configured for YouTube.
  const yt = youtubePolicy(focusCfg);
  const youtubeRows: string[] = [];
  if (yt.mode === 'block' && yt.blocked.length)
    youtubeRows.push(`Blocked: ${yt.blocked.join(', ')}`);
  if (yt.mode === 'study') {
    youtubeRows.push(
      yt.allowed.length > 0
        ? `Only these channels: ${yt.allowed.join(', ')}`
        : 'Study mode — no channels listed, so no videos play.',
    );
    if (yt.blocked.length)
      youtubeRows.push(`Also blocked: ${yt.blocked.join(', ')}`);
  }
  if (yt.entertainment) youtubeRows.push('Entertainment blocked');
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
              ? 'No early exit'
              : session.allow_pause
                ? 'Flexible'
                : 'No pause'}
          </p>
          {session.force_quit && (
            <p className="force-quit-flag" role="note">
              Force quit during session
            </p>
          )}
          {tamper && (
            <p className="small" role="note">
              {tamper}
            </p>
          )}
          {lockText && (
            <p className="small" role="status">
              Locked: {lockText}
            </p>
          )}
          {frozen && (
            <p className="small" role="status">
              Frozen{frozenText ? `: ${frozenText}` : ''} · ends only when the
              timer ends
            </p>
          )}
          {live &&
            session.focus?.pomodoro?.enabled &&
            session.pomodoro &&
            !session.reward?.break_remaining_ms && (
              <p className="small" role="status">
                {pomodoroLabel(session.pomodoro)} ·{' '}
                {countdown(session.pomodoro.phase_remaining_ms)} to next phase
              </p>
            )}
          {live &&
            session.focus?.reward?.enabled &&
            session.reward &&
            (session.reward.credit_ms > 0 ||
              session.reward.pending_delay_ms > 0 ||
              session.reward.break_remaining_ms > 0) && (
              <p className="small" role="status">
                {session.reward.break_remaining_ms > 0
                  ? `Reward break · ${countdown(session.reward.break_remaining_ms)} left`
                  : session.reward.pending_delay_ms > 0
                    ? `Break starts in ${countdown(session.reward.pending_delay_ms)}`
                    : `Earned break credit: ${countdown(session.reward.credit_ms)}`}
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
        {session.status === 'running' &&
          session.allow_pause &&
          !committed &&
          !frozen && (
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
        {session.status === 'running' &&
          session.focus?.reward?.enabled &&
          (session.reward?.credit_ms ?? 0) > 0 &&
          !(session.reward?.break_remaining_ms ?? 0) &&
          !(session.reward?.pending_delay_ms ?? 0) &&
          (!session.pomodoro || session.pomodoro.phase === 'focus') && (
            <button
              className="secondary"
              disabled={busy}
              onClick={(e) => {
                e.stopPropagation();
                void execute({
                  type: 'take_reward_break',
                  id: session.id,
                });
              }}
            >
              <Timer size={15} />
              Take break
            </button>
          )}
        {session.status === 'running' &&
          (session.reward?.pending_delay_ms ?? 0) > 0 && (
            <button
              className="secondary"
              disabled={busy}
              onClick={(e) => {
                e.stopPropagation();
                void execute({
                  type: 'cancel_delayed_break',
                  id: session.id,
                });
              }}
            >
              Cancel break
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
          {gated && (
            <section className="detail-block" aria-label="Lock">
              <h4>Lock</h4>
              <p className="small">
                Ending, pausing, or deleting early needs every lock below to
                pass. The lock lifts on its own when the timer ends, so a
                forgotten password never locks you out permanently.
              </p>
              {session.lock?.password_hash && (
                <label>
                  Password
                  <input
                    type="password"
                    value={unlockPassword}
                    onChange={(e) => setUnlockPassword(e.target.value)}
                    autoComplete="off"
                  />
                </label>
              )}
              {(session.lock?.challenge_length ?? 0) > 0 &&
                session.lock_state?.challenge && (
                  <label>
                    Type this challenge exactly:{' '}
                    <code>{session.lock_state.challenge}</code>
                    <input
                      value={unlockChallenge}
                      onChange={(e) => setUnlockChallenge(e.target.value)}
                      autoComplete="off"
                    />
                  </label>
                )}
              {(session.lock?.delay_minutes ?? 0) > 0 && (
                <>
                  {lockDelayPending && (
                    <p className="small">
                      Delay for {lockDelayFor} unlocks in{' '}
                      {lockDelay.minutesLeft} min. The block stays active during
                      the delay.
                    </p>
                  )}
                  {lockDelayDone && (
                    <p className="small">
                      Delay done — you can {lockDelayFor} now.
                    </p>
                  )}
                  <div className="row controls">
                    {!lockDelayPending && !lockDelayDone && (
                      <>
                        <button
                          type="button"
                          className="secondary"
                          disabled={busy}
                          onClick={() =>
                            void execute({
                              type: 'request_lock_delay',
                              id: session.id,
                              for_action: 'end',
                            })
                          }
                        >
                          Request end delay ({session.lock!.delay_minutes} min)
                        </button>
                        {session.status === 'running' &&
                          session.allow_pause &&
                          !committed &&
                          !frozen && (
                            <button
                              type="button"
                              className="secondary"
                              disabled={busy}
                              onClick={() =>
                                void execute({
                                  type: 'request_lock_delay',
                                  id: session.id,
                                  for_action: 'pause',
                                })
                              }
                            >
                              Request pause delay
                            </button>
                          )}
                        <button
                          type="button"
                          className="secondary"
                          disabled={busy}
                          onClick={() =>
                            void execute({
                              type: 'request_lock_delay',
                              id: session.id,
                              for_action: 'delete',
                            })
                          }
                        >
                          Request delete delay
                        </button>
                      </>
                    )}
                    {lockDelayPending && (
                      <button
                        type="button"
                        className="secondary"
                        disabled={busy}
                        onClick={() =>
                          void execute({
                            type: 'cancel_lock_delay',
                            id: session.id,
                          })
                        }
                      >
                        Cancel delay
                      </button>
                    )}
                  </div>
                </>
              )}
              <div className="row controls">
                {session.status === 'running' &&
                  session.allow_pause &&
                  !committed &&
                  !frozen && (
                    <button
                      type="button"
                      className="secondary"
                      disabled={busy}
                      onClick={() =>
                        void execute({
                          type: 'pause_session',
                          id: session.id,
                          ...proofs,
                        })
                      }
                    >
                      Pause with proofs
                    </button>
                  )}
                <button
                  type="button"
                  className="secondary"
                  disabled={busy}
                  onClick={() =>
                    void execute({
                      type: 'end_session',
                      id: session.id,
                      ...proofs,
                    })
                  }
                >
                  End early with proofs
                </button>
                <button
                  type="button"
                  className="secondary danger"
                  disabled={busy}
                  onClick={() =>
                    void execute({
                      type: 'delete_session',
                      id: session.id,
                      ...proofs,
                    })
                  }
                >
                  Delete with proofs
                </button>
              </div>
            </section>
          )}
          {frozen && live && (
            <section className="detail-block" aria-label="Frozen">
              <h4>Frozen</h4>
              <p className="small">
                This session freezes the whole computer: every app and website
                not on its allowlists is blocked, and it cannot be paused, ended
                early, or deleted. There is no emergency unlock — the lock ends
                on its own when the timer runs out, on wall-clock time.
              </p>
            </section>
          )}
          <section className="detail-block" aria-label="About this session">
            <h4>About this session</h4>
            <dl className="detail-list">
              <div>
                <dt>Length</dt>
                <dd>{session.duration_minutes} minutes</dd>
              </div>
              <div>
                <dt>Session type</dt>
                <dd>{sessionTypeLabel(session)}</dd>
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
              {youtubeRows.length > 0 && (
                <div>
                  <dt>YouTube</dt>
                  <dd>{youtubeRows.join(' · ')}</dd>
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
              Connecting to your browser… Open its Preelock extension and click
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
              {live &&
                needsCamera &&
                report?.camera_status !== 'person_detected' && (
                  <>
                    <p className="muted">
                      {session.status === 'paused'
                        ? 'Resume the session to restart camera checks.'
                        : session.focus?.camera
                          ? 'Camera checks retry automatically. Time resumes when you return and verification succeeds.'
                          : 'Camera is stopped. Start it to resume camera verification.'}
                    </p>
                    <button
                      className="secondary"
                      disabled={busy}
                      onClick={() => {
                        if (cameraBlocked)
                          void api.openSetup('camera_settings');
                        else if (cameraPermission !== 'granted')
                          void api
                            .requestCamera()
                            .then(() => onSetupRefresh?.());
                        else
                          void execute({
                            type: 'enable_camera',
                            id: session.id,
                          });
                      }}
                    >
                      {cameraBlocked
                        ? 'Open camera settings'
                        : cameraPermission !== 'granted'
                          ? 'Allow camera'
                          : session.focus?.camera
                            ? 'Retry camera'
                            : 'Start camera'}
                    </button>
                  </>
                )}
            </div>
          )}
        </div>
      )}
      {readyOpen && (
        <Modal title="Before you start" onClose={() => setReadyOpen(false)}>
          <div className="dialog-stack">
            {focusCfg && (
              <SessionReview
                focus={focusCfg}
                title={session.title}
                timing={{
                  kind: 'immediate',
                  minutes: session.duration_minutes,
                }}
                appNames={appNames}
                browserName={reviewBrowserName}
                browserConnected={browserOk}
                cameraPermission={setup?.camera_permission}
                platform={browserSetup?.platform}
                safariWarning={safariWarning}
                lockText={lockText}
              />
            )}
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
                      Camera access is blocked for Preelock, so this session
                      can't start its camera check.
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
                        // Fetch this browser's status now, and also refresh
                        // the popup's own sessionSetup so the readiness check
                        // flips to Ready if the extension is already connected.
                        // (Previously only guideSetup was updated, leaving
                        // browserReady false while BrowserSetup saw a live
                        // connection — the empty broken dialog.)
                        api
                          .focusSetup(target)
                          .then((fresh) => {
                            setGuideSetup(fresh);
                            setSessionSetup(fresh);
                          })
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
