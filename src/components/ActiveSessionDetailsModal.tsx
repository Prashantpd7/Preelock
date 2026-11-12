import { useEffect, useState } from 'react';
import { useAppStore } from '../state/store';
import { api } from '../services/api';
import { countdown } from '../services/analytics';
import { frozenLabel, lockLabel, pomodoroLabel, tamperNote } from '../types';
import type { FocusSetup } from '../types';
import {
  effectiveWebsiteBlocking,
  elapsedMs,
  formatDateTime,
  formatHMS,
  isLiveSessionStatus,
  liveTimeStats,
  resolveAppName,
  scheduledEndMs,
  sessionTypeLabel,
  verificationText,
  workAppIds,
  youtubePolicy,
} from '../services/activeSession';
import { Modal } from './SettingsControls';

/** Read-only details popup for the live session. It reads the
 *  same shared store as the banner (refreshed from the backend
 *  every second), so pause / resume / timer changes show up live;
 *  the app name list comes from a fresh `focusSetup` fetch when
 *  the popup opens. Nothing here mutates anything: closing the
 *  popup only closes the dialog. Only what is actually configured
 *  in the session is shown — unselected features render no row
 *  and empty sections are omitted entirely. */
export function ActiveSessionDetailsModal({
  sessionId,
  onClose,
}: {
  sessionId: string;
  onClose: () => void;
}) {
  const data = useAppStore((s) => s.data);
  const session = data.sessions.find((s) => s.id === sessionId);
  const [setup, setSetup] = useState<FocusSetup | null>(null);
  // Live clock for the elapsed line (same pattern as the banner's
  // grace clock): the render stays pure, Date.now() lives here.
  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  // App and browser names for this session's browser, fetched
  // when the popup opens — it can differ from the most recently
  // used browser elsewhere. The browser id is the only thing
  // this fetch depends on: the session object itself is a new
  // identity on every store refresh, so it must not be a dep.
  const browserApp = session?.focus?.browser_app ?? '';
  const hasSession = !!session;
  useEffect(() => {
    if (!hasSession) return;
    let active = true;
    api
      .focusSetup(browserApp)
      .then((v) => {
        if (active) setSetup(v);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [hasSession, browserApp]);
  if (!session || !isLiveSessionStatus(session.status)) return null;
  const focus = session.focus;
  const appNames: Record<string, string> = {};
  for (const app of setup?.applications ?? []) appNames[app.id] = app.name;
  const graceUntilMs = session.grace_until_ms ?? null;
  const graceRemainingSecs =
    session.status === 'running' && graceUntilMs
      ? Math.max(0, Math.ceil((graceUntilMs - nowMs) / 1000))
      : 0;
  const inGrace = graceRemainingSecs > 0;
  const endMs = scheduledEndMs(session);
  const elapsed = elapsedMs(session, nowMs);
  const tamper = tamperNote(session);
  const statusLabel =
    session.status === 'running'
      ? 'Running'
      : session.status === 'paused'
        ? 'Paused'
        : 'Preparing';
  const statusPill = session.status === 'running' ? 'ok' : 'warn';
  const apps = workAppIds(focus);
  // Live focus totals: the engine credits productive time and
  // fills the attention buckets on every tick while the session
  // runs, and the store refreshes every second, so these move
  // live. Only meaningful when focus checks are on — a
  // timer-only session has no focus data, so the section is
  // omitted for it, like every other unconfigured row here.
  const showTimeSoFar = focus?.attention?.enabled === true;
  const timeStats = liveTimeStats(session);
  const blockedFolders = focus?.blocked_folders ?? [];
  const blockedTitles = focus?.blocked_titles ?? [];
  const websiteMode = effectiveWebsiteBlocking(focus);
  const domains = focus?.domains ?? [];
  const workDomains = focus?.attention?.work_domains ?? [];
  const exceptions = focus?.exceptions ?? [];
  const yt = youtubePolicy(focus);
  const needsCamera =
    focus?.camera === true ||
    focus?.attention?.verification === 'camera_and_activity';
  // Only what is actually configured shows in this popup: every
  // row below renders only when its feature is on / non-empty,
  // and a section with no rows left is omitted entirely.
  const lockText = lockLabel(session.lock);
  const showFocusChecks = focus?.attention?.enabled === true;
  const showCamera = focus?.camera === true;
  const showPomodoro = focus?.pomodoro?.enabled === true;
  const showRewards = focus?.reward?.enabled === true;
  const showFrozen = focus?.frozen?.enabled === true;
  const showLocks = lockText != null;
  const showOtherRestrictions =
    showFocusChecks ||
    showCamera ||
    showPomodoro ||
    showRewards ||
    showFrozen ||
    showLocks;
  const blockOtherNote = [
    focus?.activity_consent ? 'Block other apps: On' : null,
    focus?.mode === 'strict'
      ? 'Strict mode — every app outside this list is blocked.'
      : null,
  ]
    .filter((s): s is string => !!s)
    .join(' · ');
  return (
    <Modal title={session.title} onClose={onClose}>
      <div className="dialog-stack">
        <div>
          <span className={`status-pill ${statusPill}`}>{statusLabel}</span>
          {session.status === 'preparing' && (
            <span className="small muted"> · waiting for browser</span>
          )}
          {inGrace && <span className="small muted"> · starting soon</span>}
          <p className="active-session-timer" style={{ marginTop: 12 }}>
            {inGrace
              ? `Starts in ${graceRemainingSecs}s`
              : formatHMS(session.remaining_ms)}
          </p>
          <p className="small muted" style={{ margin: 0 }}>
            {inGrace
              ? 'Blocking begins when the countdown ends.'
              : 'Remaining in this session.'}
          </p>
        </div>

        <section className="detail-block" aria-label="Session overview">
          <h4>Session overview</h4>
          <dl className="detail-list">
            <div>
              <dt>Session type</dt>
              <dd>{sessionTypeLabel(session)}</dd>
            </div>
            <div>
              <dt>Started</dt>
              <dd>
                {session.started_at != null
                  ? formatDateTime(session.started_at)
                  : 'Not started yet'}
              </dd>
            </div>
            <div>
              <dt>Ends</dt>
              <dd>{endMs != null ? formatDateTime(endMs) : '—'}</dd>
            </div>
            <div>
              <dt>Duration</dt>
              <dd>{session.duration_minutes} minutes</dd>
            </div>
            <div>
              <dt>Elapsed</dt>
              <dd>{elapsed != null ? formatHMS(elapsed) : '—'}</dd>
            </div>
          </dl>
          {tamper && (
            <p className="small" role="note">
              {tamper}
            </p>
          )}
          {session.force_quit && (
            <p className="force-quit-flag" role="note">
              Force quit during session
            </p>
          )}
        </section>

        {showTimeSoFar && (
          <section className="detail-block" aria-label="Time so far">
            <h4>Time so far</h4>
            <div className="report-metrics">
              <span>Focused {countdown(timeStats.focusedMs)}</span>
              <span>Distracted {countdown(timeStats.distractedMs)}</span>
              <span>Away {countdown(timeStats.awayMs)}</span>
              <span>Total {countdown(timeStats.totalMs)}</span>
            </div>
          </section>
        )}

        <section className="detail-block" aria-label="Selected apps">
          <h4>Selected apps</h4>
          {apps.length === 0 &&
            blockedFolders.length === 0 &&
            blockedTitles.length === 0 && (
              <p className="small muted">
                No work apps selected for this session.
              </p>
            )}
          {(apps.length > 0 ||
            blockedFolders.length > 0 ||
            blockedTitles.length > 0) && (
            <ul className="detail-items">
              {apps.map((id) => (
                <li key={id}>
                  <span>{resolveAppName(id, appNames)}</span>
                  <span className="status-pill ok">Allowed</span>
                </li>
              ))}
              {blockedFolders.map((folder) => (
                <li key={`folder:${folder}`}>
                  <span>{folder}</span>
                  <span className="status-pill blocked">Blocked folder</span>
                </li>
              ))}
              {blockedTitles.map((title) => (
                <li key={`title:${title}`}>
                  <span>{title}</span>
                  <span className="status-pill blocked">Blocked title</span>
                </li>
              ))}
            </ul>
          )}
          {blockOtherNote && <p className="small muted">{blockOtherNote}</p>}
        </section>

        {websiteMode !== 'none' && (
          <section className="detail-block" aria-label="Website restrictions">
            <h4>Website restrictions</h4>
            {focus && !focus.general_internet && (
              <p className="small" role="note">
                All internet access is blocked during this session.
              </p>
            )}
            {websiteMode === 'blocklist' && (
              <>
                <p className="small">Block distracting websites</p>
                {domains.length > 0 ? (
                  <ul className="chip-list">
                    {domains.map((d) => (
                      <li key={d} className="chip blocked">
                        {d}
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p className="small muted">No websites listed to block.</p>
                )}
              </>
            )}
            {websiteMode === 'allowlist' && (
              <>
                <p className="small">Block all websites except these</p>
                {domains.length > 0 ? (
                  <ul className="chip-list">
                    {domains.map((d) => (
                      <li key={d} className="chip allowed">
                        {d}
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p className="small" role="note">
                    No websites listed — all websites are blocked.
                  </p>
                )}
                <p className="small muted">Every other website is blocked.</p>
              </>
            )}
            {workDomains.length > 0 && (
              <>
                <p className="small">Count as focused work</p>
                <ul className="chip-list">
                  {workDomains.map((d) => (
                    <li key={d} className="chip allowed">
                      {d}
                    </li>
                  ))}
                </ul>
              </>
            )}
            {websiteMode === 'blocklist' && exceptions.length > 0 && (
              <>
                <p className="small">Always allowed</p>
                <ul className="chip-list">
                  {exceptions.map((d) => (
                    <li key={d} className="chip allowed">
                      {d}
                    </li>
                  ))}
                </ul>
              </>
            )}
          </section>
        )}

        {(yt.mode !== 'none' || yt.entertainment) && (
          <section className="detail-block" aria-label="YouTube restrictions">
            <h4>YouTube restrictions</h4>
            {yt.mode === 'study' && (
              <>
                <p className="small">Only the listed channels are allowed.</p>
                {yt.allowed.length > 0 ? (
                  <ul className="chip-list">
                    {yt.allowed.map((c) => (
                      <li key={c} className="chip allowed">
                        {c}
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p className="small muted">
                    No channels listed — no YouTube videos play.
                  </p>
                )}
              </>
            )}
            {yt.mode === 'block' && (
              <>
                <p className="small">
                  These channels and their videos are blocked.
                </p>
                <ul className="chip-list">
                  {yt.blocked.map((c) => (
                    <li key={c} className="chip blocked">
                      {c}
                    </li>
                  ))}
                </ul>
              </>
            )}
            {yt.mode === 'study' && yt.blocked.length > 0 && (
              <>
                <p className="small">Also blocked</p>
                <ul className="chip-list">
                  {yt.blocked.map((c) => (
                    <li key={c} className="chip blocked">
                      {c}
                    </li>
                  ))}
                </ul>
              </>
            )}
            {yt.entertainment && (
              <p className="small">
                Entertainment content (comedy, pranks, vlogs, funny/animal
                clips) is blocked.
              </p>
            )}
            {yt.watchLater && (
              <p className="small muted">Watch Later exception is enabled.</p>
            )}
          </section>
        )}

        {showOtherRestrictions && (
          <section className="detail-block" aria-label="Other restrictions">
            <h4>Other restrictions</h4>
            <dl className="detail-list">
              {showFocusChecks && (
                <div>
                  <dt>Focus checks</dt>
                  <dd>{verificationText(focus)}</dd>
                </div>
              )}
              {showCamera && (
                <div>
                  <dt>Camera</dt>
                  <dd>
                    On — checks you&apos;re at your desk.
                    {needsCamera && setup && (
                      <> Permission: {setup.camera_permission}.</>
                    )}
                  </dd>
                </div>
              )}
              {showPomodoro && focus?.pomodoro && (
                <div>
                  <dt>Pomodoro</dt>
                  <dd>
                    {`${focus.pomodoro.focus_minutes}m focus · ${focus.pomodoro.short_break_minutes}m short break · ${focus.pomodoro.long_break_minutes}m long break · long break every ${focus.pomodoro.cycles_before_long_break} cycles${
                      session.pomodoro
                        ? ` · now: ${pomodoroLabel(session.pomodoro)} (${countdown(session.pomodoro.phase_remaining_ms)} left)`
                        : ''
                    }`}
                  </dd>
                </div>
              )}
              {showRewards && focus?.reward && (
                <div>
                  <dt>Reward breaks</dt>
                  <dd>
                    {`${focus.reward.earn_focus_minutes}m focus earns ${focus.reward.earn_break_minutes}m · max credit ${focus.reward.max_credit_minutes}m · longest break ${focus.reward.max_break_minutes}m${
                      session.reward
                        ? session.reward.break_remaining_ms > 0
                          ? ` · break running (${countdown(session.reward.break_remaining_ms)} left)`
                          : session.reward.credit_ms > 0
                            ? ` · earned credit: ${countdown(session.reward.credit_ms)}`
                            : ''
                        : ''
                    }`}
                  </dd>
                </div>
              )}
              {showFrozen && (
                <div>
                  <dt>Frozen</dt>
                  <dd>
                    {frozenLabel(focus) ?? ''} · ends only when the timer ends
                  </dd>
                </div>
              )}
              {showLocks && (
                <div>
                  <dt>Locks</dt>
                  <dd>{lockText}</dd>
                </div>
              )}
            </dl>
          </section>
        )}
      </div>
    </Modal>
  );
}
