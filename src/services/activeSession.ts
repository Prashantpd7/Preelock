import type {
  BrowserStatus,
  FocusConfig,
  Session,
  SessionStatus,
} from '../types';

/** A session counts as live (shown in the global banner) while it is
 *  running, paused, or preparing — the same set the Dashboard used. */
export function isLiveSessionStatus(status: SessionStatus): boolean {
  return ['running', 'paused', 'preparing'].includes(status);
}

/** The single live session, if any. The backend guards a single
 *  live session, so the first match is the active one. */
export function liveSession(sessions: Session[]): Session | undefined {
  return sessions.find((s) => isLiveSessionStatus(s.status));
}

/** Human label for a session's origin, from the kind the backend
 *  records at creation and persists on the session. Sessions
 *  saved before the backend recorded it have no `kind`, so the
 *  label falls back to `schedule_id` — that still proves a
 *  scheduled session, while an old one-shot session (whose timer
 *  record was purged) shows as a plain focus session. */
export function sessionTypeLabel(session: Session): string {
  switch (session.kind) {
    case 'scheduled':
      return 'Scheduled session';
    case 'one_shot':
      return 'One-shot session';
    case 'focus':
      return 'Focus session';
    default:
      return session.schedule_id ? 'Scheduled session' : 'Focus session';
  }
}

/** Live focus totals for a session, from the same fields
 *  SessionCard's "Time so far" / "Final result" uses: the
 *  productive time the engine credits plus the attention
 *  buckets, which the engine updates on every tick while the
 *  session runs. Only meaningful when focus checks are on. */
export function liveTimeStats(session: Session): {
  focusedMs: number;
  distractedMs: number;
  awayMs: number;
  totalMs: number;
} {
  const focusedMs = session.focused_ms;
  const distractedMs = session.attention?.distracted_ms ?? 0;
  const awayMs = session.attention?.absent_ms ?? 0;
  return {
    focusedMs,
    distractedMs,
    awayMs,
    totalMs: focusedMs + distractedMs + awayMs,
  };
}

/** Whether this session needs the browser extension connected.
 *  Mirrors the SessionCard / `setupProblem` rule, including the
 *  frozen-session clause. */
export function sessionNeedsBrowser(focus: FocusConfig | undefined): boolean {
  if (!focus) return false;
  return (
    !focus.general_internet ||
    (focus.domains?.length ?? 0) > 0 ||
    (focus.mode !== 'standard' && !!focus.browser_app) ||
    (focus.attention?.work_domains?.length ?? 0) > 0 ||
    (focus.youtube_blocked_channels?.length ?? 0) > 0 ||
    (focus.youtube_allowed_channels?.length ?? 0) > 0 ||
    !!focus.youtube_study_mode ||
    (!!focus.frozen?.enabled && !!focus.browser_app)
  );
}

/** The website-blocking mode that actually applies: `auto` follows
 *  the session mode (strict → allowlist, anything else →
 *  blocklist), matching `FocusConfig` handling in the backend. */
export function effectiveWebsiteBlocking(
  focus: FocusConfig | undefined,
): 'blocklist' | 'allowlist' | 'none' {
  const blocking = focus?.website_blocking ?? 'auto';
  if (blocking === 'none') return 'none';
  if (blocking === 'auto')
    return focus?.mode === 'strict' ? 'allowlist' : 'blocklist';
  return blocking;
}

/** Work apps for a session: the attention work list plus the
 *  session's own app list, deduplicated, in first-seen order. */
export function workAppIds(focus: FocusConfig | undefined): string[] {
  return Array.from(
    new Set([...(focus?.attention?.work_apps ?? []), ...(focus?.apps ?? [])]),
  );
}

/** Display name for an app id: a known name wins, otherwise the
 *  last dotted segment with separators turned into spaces — the
 *  same fallback SessionCard uses. */
export function resolveAppName(
  id: string,
  names?: Record<string, string>,
): string {
  if (names?.[id]) return names[id];
  const tail = id.split('.').pop() || id;
  return tail.replace(/[-_]/g, ' ').trim() || id;
}

/** "11 Nov 2026 at 4:16 PM" — the same format SessionCard uses
 *  for session start times. */
export function formatDateTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    day: 'numeric',
    month: 'short',
    year: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
}

/** HH:MM:SS, for the popup's large remaining/elapsed displays. */
export function formatHMS(ms: number): string {
  const seconds = Math.max(0, Math.ceil(ms / 1000));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const secs = seconds % 60;
  return `${hours.toString().padStart(2, '0')}:${minutes
    .toString()
    .padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
}

/** Wall-clock instant the session is scheduled to end, from its
 *  start time plus its original duration. Null when the session
 *  has not started. */
export function scheduledEndMs(session: Session): number | null {
  if (session.started_at == null) return null;
  return session.started_at + session.duration_minutes * 60_000;
}

/** Wall-clock time since the session started. Null when the
 *  session has not started. */
export function elapsedMs(session: Session, nowMs: number): number | null {
  if (session.started_at == null) return null;
  return Math.max(0, nowMs - session.started_at);
}

export type BrowserEnforcementState =
  | 'not-needed'
  | 'enforced'
  | 'connected-not-applied'
  | 'disconnected'
  | 'error'
  | 'fail-closed';

/** Whether the current session's browser rules are actually
 *  enforced, from the live bridge status. `applied_id` is the
 *  session id whose policy the extension last reported applying
 *  (see `browser.rs` / `engine.rs`); only a match means this
 *  session's rules are live in the browser. A merely-connected
 *  extension is never reported as enforcing. */
export function browserEnforcement(
  session: Session,
  browser: BrowserStatus | undefined,
): { state: BrowserEnforcementState; label: string; detail: string } {
  if (!sessionNeedsBrowser(session.focus)) {
    return {
      state: 'not-needed',
      label: 'Not needed',
      detail: 'This session does not use browser blocking.',
    };
  }
  if (session.browser_fail_closed) {
    return {
      state: 'fail-closed',
      label: 'Browser blocked',
      detail:
        'This session’s website rules were never applied, so the browser app itself stays blocked until the extension is connected.',
    };
  }
  if (!browser || !browser.connected) {
    return {
      state: 'disconnected',
      label: 'Not enforced',
      detail:
        'The browser extension is not connected, so this session’s website rules are not being enforced.',
    };
  }
  if (browser.error) {
    return {
      state: 'error',
      label: 'Not enforced',
      detail: `The browser reported an error: ${browser.error}`,
    };
  }
  if (browser.applied_id === session.id) {
    return {
      state: 'enforced',
      label: 'Enforced',
      detail:
        'The extension has applied this session’s website rules in the browser.',
    };
  }
  return {
    state: 'connected-not-applied',
    label: 'Connected — rules not applied yet',
    detail:
      'The extension is connected, but this session’s website rules have not been applied in the browser yet.',
  };
}

export type YoutubePolicy =
  | {
      mode: 'none';
      blocked: string[];
      allowed: string[];
      watchLater: boolean;
      entertainment?: boolean;
    }
  | {
      mode: 'block';
      blocked: string[];
      allowed: string[];
      watchLater: boolean;
      entertainment?: boolean;
    }
  | {
      mode: 'study';
      blocked: string[];
      allowed: string[];
      watchLater: boolean;
      entertainment?: boolean;
    };

/** The session's exact YouTube configuration. Study mode allows
 *  only the allowed list; otherwise the blocked list applies.
 *  Watch Later and entertainment blocking are reported as flags. */
export function youtubePolicy(focus: FocusConfig | undefined): YoutubePolicy {
  const blocked = focus?.youtube_blocked_channels ?? [];
  const allowed = focus?.youtube_allowed_channels ?? [];
  const watchLater = focus?.youtube_allow_watch_later ?? false;
  const ent = focus?.youtube_block_entertainment
    ? { entertainment: true as const }
    : {};
  if (focus?.youtube_study_mode)
    return { mode: 'study', blocked, allowed, watchLater, ...ent };
  if (blocked.length > 0)
    return { mode: 'block', blocked, allowed, watchLater, ...ent };
  return { mode: 'none', blocked, allowed, watchLater, ...ent };
}

/** "Type of work" label for the attention `task` value, matching
 *  the wizard's dropdown. Unknown values are shown as-is rather
 *  than guessed. */
export function typeOfWorkLabel(task: string | undefined): string {
  switch (task) {
    case 'coding':
      return 'Coding';
    case 'reading':
      return 'Reading / study';
    case 'lecture':
      return 'Watching a lesson';
    case 'writing':
      return 'Writing';
    case 'other':
      return 'General work';
    default:
      return task ? task : 'General work';
  }
}

/** Display name for an attention state, matching SessionCard. */
export function attentionStateLabel(state: string): string {
  const names: Record<string, string> = {
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
  return names[state] ?? state;
}

/** What the session's focus checks do, matching SessionCard. */
export function verificationText(focus: FocusConfig | undefined): string {
  if (!focus?.attention?.enabled) return 'Timer only — no focus checks.';
  return focus.attention.verification === 'camera_and_activity'
    ? "Checks your apps and that you're at your desk."
    : 'Checks which app you are using.';
}
