import { useState } from 'react';
import { frozenLabel } from '../types';
import type { FocusConfig } from '../types';
import {
  effectiveWebsiteBlocking,
  formatDateTime,
  resolveAppName,
  typeOfWorkLabel,
  verificationText,
  workAppIds,
  youtubePolicy,
} from '../services/activeSession';
import { sessionNeedsExtension } from '../services/browserSupport';
import { formatDays } from '../services/schedules';

/** Timing the review shows. The caller passes the timing that
 *  matches its own workflow — immediate start, a schedule being
 *  created (days/times picked in the dialog), or a one-shot
 *  (date/time picked in the dialog). The review never invents
 *  timing: a one-shot with no time picked yet says so. */
export type SessionReviewTiming =
  | { kind: 'immediate'; minutes: number }
  | {
      kind: 'schedule';
      minutes: number;
      days: number[];
      startTime: string;
      endTime: string;
    }
  | { kind: 'oneshot'; minutes: number; fireAtMs: number | null };

export interface SessionReviewProps {
  /** The exact FocusConfig that will be submitted. The review
   *  reads it directly — no separate model, no copied values. */
  focus: FocusConfig;
  /** Session name as it stands now (live dialog state for
   *  schedule/one-shot, where the name is editable). */
  title: string;
  timing: SessionReviewTiming;
  appNames?: Record<string, string>;
  /** Resolved display name of the selected browser, '' when none. */
  browserName?: string;
  /** Whether the selected browser's extension is connected and
   *  reporting from that browser (caller's readiness check). */
  browserConnected?: boolean;
  /** Camera permission string from FocusSetup, e.g. 'granted'. */
  cameraPermission?: string;
  /** Platform string from FocusSetup ('windows' | 'darwin' ...).
   *  Safari warnings and camera-permission warnings are never
   *  shown on Windows: Safari is macOS-only and camera checks
   *  do not exist on Windows. */
  platform?: string;
  /** Caller-computed: Safari is selected and the session needs
   *  the extension. Ignored on Windows. */
  safariWarning?: boolean;
  /** Lock summary text (schedule dialog), null when no lock. */
  lockText?: string | null;
  /** Clock for the expected end time; defaults to now. Pinned
   *  in state so re-renders do not shift the displayed time. */
  nowMs?: number;
}

const CHIP_PREVIEW = 8;

function ExpandableChips({
  items,
  tone,
  label,
}: {
  items: string[];
  tone: 'blocked' | 'allowed';
  label: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const shown = expanded ? items : items.slice(0, CHIP_PREVIEW);
  return (
    <>
      <ul className="chip-list">
        {shown.map((item) => (
          <li key={item} className={`chip ${tone}`}>
            {item}
          </li>
        ))}
      </ul>
      {items.length > CHIP_PREVIEW && (
        <button
          type="button"
          className="secondary"
          onClick={() => setExpanded((v) => !v)}
          aria-expanded={expanded}
        >
          {expanded ? 'Show less' : `Show all ${items.length} ${label}`}
        </button>
      )}
    </>
  );
}

function parseTimeMinutes(value: string): number | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(value.trim());
  if (!match) return null;
  const hours = Number(match[1]);
  const minutes = Number(match[2]);
  if (hours > 23 || minutes > 59) return null;
  return hours * 60 + minutes;
}

/**
 * Compact review of the session about to be created or started,
 * shown inside the confirmation popup before the user confirms.
 * It reads the exact FocusConfig that will be submitted, plus
 * the timing the caller is about to submit — nothing is copied
 * into a second model and nothing is hardcoded.
 *
 * Only what is actually configured is shown: optional features
 * (Pomodoro, rewards, frozen, locks, YouTube, browser) render
 * no row when they are off or empty. Website blocking is the
 * one exception the brief calls out explicitly: when it is off,
 * a compact "Website blocking: Off" status is shown, because
 * that distinction matters before starting.
 */
export function SessionReview({
  focus,
  title,
  timing,
  appNames,
  browserName,
  browserConnected,
  cameraPermission,
  platform,
  safariWarning,
  lockText,
  nowMs,
}: SessionReviewProps) {
  const [frozenNowMs] = useState(() => nowMs ?? Date.now());
  const isWindows = platform === 'windows';
  const typeLabel =
    timing.kind === 'schedule'
      ? 'Scheduled session'
      : timing.kind === 'oneshot'
        ? 'One-shot session'
        : 'Focus session';
  const strict = focus.mode === 'strict';

  // Timing rows. A schedule shows its days and window instead of
  // pretending a session is already running; a one-shot shows its
  // picked start, or says so when none is picked yet.
  let startLabel: string;
  let endLabel: string;
  let durationLabel: string;
  if (timing.kind === 'schedule') {
    const startMin = parseTimeMinutes(timing.startTime);
    const endMin = parseTimeMinutes(timing.endTime);
    let windowMinutes: number | null = null;
    let overnight = false;
    if (startMin !== null && endMin !== null) {
      windowMinutes = endMin - startMin;
      if (windowMinutes <= 0) {
        windowMinutes += 24 * 60;
        overnight = true;
      }
    }
    startLabel = `${formatDays(timing.days)} · ${timing.startTime}`;
    endLabel = timing.endTime + (overnight ? ' (next day)' : '');
    durationLabel = windowMinutes !== null ? `${windowMinutes} minutes` : '—';
  } else if (timing.kind === 'oneshot') {
    startLabel =
      timing.fireAtMs !== null
        ? formatDateTime(timing.fireAtMs)
        : 'Pick a date and time';
    endLabel =
      timing.fireAtMs !== null
        ? formatDateTime(timing.fireAtMs + timing.minutes * 60_000)
        : '—';
    durationLabel = `${timing.minutes} minutes`;
  } else {
    startLabel = 'Immediately';
    endLabel = formatDateTime(frozenNowMs + timing.minutes * 60_000);
    durationLabel = `${timing.minutes} minutes`;
  }

  const apps = workAppIds(focus);
  const blockedFolders = focus.blocked_folders ?? [];
  const blockedTitles = focus.blocked_titles ?? [];
  const websiteMode = effectiveWebsiteBlocking(focus);
  const domains = focus.domains ?? [];
  const workDomains = focus.attention?.work_domains ?? [];
  const exceptions = focus.exceptions ?? [];
  const yt = youtubePolicy(focus);
  const showYoutube = yt.mode !== 'none' || yt.watchLater;
  const browserNeeded = sessionNeedsExtension(focus);
  const showBrowser = browserNeeded || !!browserName;
  const showSafariWarning = !!safariWarning && !isWindows;
  const needsCamera =
    focus.camera === true ||
    focus.attention?.verification === 'camera_and_activity';
  const showCameraWarning =
    !isWindows && needsCamera && cameraPermission !== 'granted';
  const blockOtherNote = [
    focus.activity_consent ? 'Block other apps: On' : null,
    strict ? 'Strict mode — every app outside this list is blocked.' : null,
  ]
    .filter((s): s is string => !!s)
    .join(' · ');

  return (
    <>
      <section className="detail-block" aria-label="Session overview">
        <h4>Session overview</h4>
        <dl className="detail-list">
          <div>
            <dt>Name</dt>
            <dd>{title || 'Untitled session'}</dd>
          </div>
          <div>
            <dt>Type</dt>
            <dd>{typeLabel}</dd>
          </div>
          <div>
            <dt>Duration</dt>
            <dd>{durationLabel}</dd>
          </div>
          <div>
            <dt>Start</dt>
            <dd>{startLabel}</dd>
          </div>
          <div>
            <dt>End</dt>
            <dd>{endLabel}</dd>
          </div>
          <div>
            <dt>Mode</dt>
            <dd>{strict ? 'Strict' : 'Standard'}</dd>
          </div>
        </dl>
      </section>

      <section className="detail-block" aria-label="Monitoring">
        <h4>Monitoring</h4>
        <dl className="detail-list">
          {!isWindows && (
            <div>
              <dt>Camera</dt>
              <dd>
                {focus.camera ? "On — checks you're at your desk." : 'Off'}
              </dd>
            </div>
          )}
          <div>
            <dt>Focus checks</dt>
            <dd>
              {focus.attention?.enabled
                ? `${verificationText(focus)}${
                    focus.attention.task !== 'other'
                      ? ` Type of work: ${typeOfWorkLabel(focus.attention.task)}.`
                      : ''
                  } Wait before labeling a distraction: ${focus.attention.grace_seconds}s.`
                : 'Off — timer only, no focus checks.'}
            </dd>
          </div>
          {focus.pomodoro?.enabled && (
            <div>
              <dt>Pomodoro</dt>
              <dd>
                {`${focus.pomodoro.focus_minutes}m focus · ${focus.pomodoro.short_break_minutes}m short break · ${focus.pomodoro.long_break_minutes}m long break · long break every ${focus.pomodoro.cycles_before_long_break} cycles`}
              </dd>
            </div>
          )}
          {focus.reward?.enabled && (
            <div>
              <dt>Reward breaks</dt>
              <dd>
                {`${focus.reward.earn_focus_minutes}m focus earns ${focus.reward.earn_break_minutes}m break · max credit ${focus.reward.max_credit_minutes}m`}
              </dd>
            </div>
          )}
          {focus.frozen?.enabled && (
            <div>
              <dt>Frozen</dt>
              <dd>
                {frozenLabel(focus) ?? ''} · ends only when the timer ends
              </dd>
            </div>
          )}
          {lockText && (
            <div>
              <dt>Locks</dt>
              <dd>{lockText}</dd>
            </div>
          )}
        </dl>
        {showCameraWarning && (
          <p className="small" role="alert">
            Camera access is not granted — allow camera access before the
            session starts.
          </p>
        )}
      </section>

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

      <section className="detail-block" aria-label="Website restrictions">
        <h4>Website restrictions</h4>
        {websiteMode === 'none' && (
          <p className="small muted">Website blocking: Off</p>
        )}
        {websiteMode !== 'none' && (
          <>
            {focus && !focus.general_internet && (
              <p className="small" role="note">
                All internet access is blocked during this session.
              </p>
            )}
            {websiteMode === 'blocklist' && (
              <>
                <p className="small">Block distracting websites</p>
                {domains.length > 0 ? (
                  <ExpandableChips
                    items={domains}
                    tone="blocked"
                    label="websites"
                  />
                ) : (
                  <p className="small muted">No websites listed to block.</p>
                )}
              </>
            )}
            {websiteMode === 'allowlist' && (
              <>
                <p className="small">Block all websites except these</p>
                {domains.length > 0 ? (
                  <ExpandableChips
                    items={domains}
                    tone="allowed"
                    label="websites"
                  />
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
                <ExpandableChips
                  items={workDomains}
                  tone="allowed"
                  label="websites"
                />
              </>
            )}
            {websiteMode === 'blocklist' && exceptions.length > 0 && (
              <>
                <p className="small">Always allowed</p>
                <ExpandableChips
                  items={exceptions}
                  tone="allowed"
                  label="websites"
                />
              </>
            )}
          </>
        )}
      </section>

      {showYoutube && (
        <section className="detail-block" aria-label="YouTube restrictions">
          <h4>YouTube restrictions</h4>
          {yt.mode === 'study' && (
            <>
              <p className="small">Only the listed channels are allowed.</p>
              {yt.allowed.length > 0 ? (
                <ExpandableChips
                  items={yt.allowed}
                  tone="allowed"
                  label="channels"
                />
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
              <ExpandableChips
                items={yt.blocked}
                tone="blocked"
                label="channels"
              />
            </>
          )}
          {yt.mode === 'study' && yt.blocked.length > 0 && (
            <>
              <p className="small">Also blocked</p>
              <ExpandableChips
                items={yt.blocked}
                tone="blocked"
                label="channels"
              />
            </>
          )}
          {yt.mode === 'none' && (
            <p className="small muted">No channel restrictions.</p>
          )}
          <p className="small muted">
            Watch Later: {yt.watchLater ? 'Allowed' : 'Off'}
          </p>
        </section>
      )}

      {showBrowser && (
        <section className="detail-block" aria-label="Browser">
          <h4>Browser</h4>
          <dl className="detail-list">
            <div>
              <dt>Browser</dt>
              <dd>{browserName || 'None selected'}</dd>
            </div>
            <div>
              <dt>Extension</dt>
              <dd>
                <span
                  className={`status-pill ${browserConnected ? 'ok' : 'warn'}`}
                >
                  {browserConnected ? 'Connected' : 'Not connected'}
                </span>
              </dd>
            </div>
            <div>
              <dt>Ready</dt>
              <dd>
                {!browserNeeded
                  ? 'Not needed for this session.'
                  : browserConnected
                    ? 'Ready for this session.'
                    : 'Not ready — the extension is not connected.'}
              </dd>
            </div>
          </dl>
          {browserNeeded && !browserConnected && (
            <p className="small" role="alert">
              Browser extension is not connected — connect it before the session
              starts.
            </p>
          )}
          {!browserName && browserNeeded && (
            <p className="small" role="alert">
              No browser detected — open Comet, Chrome, Edge, Brave or Vivaldi,
              then refresh apps.
            </p>
          )}
          {showSafariWarning && (
            <p className="small" role="alert">
              Safari has no Preelock extension, so it can&apos;t enforce this
              session&apos;s website rules. Choose a supported browser.
            </p>
          )}
        </section>
      )}
    </>
  );
}
