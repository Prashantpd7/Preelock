export type SessionStatus =
  | 'ready'
  | 'preparing'
  | 'running'
  | 'paused'
  | 'completed'
  | 'ended'
  | 'interrupted';
export interface Session {
  id: string;
  title: string;
  duration_minutes: number;
  remaining_ms: number;
  focused_ms: number;
  allow_pause: boolean;
  status: SessionStatus;
  started_at: number | null;
  ended_at: number | null;
  deadline_ms: number | null;
  /** Legacy record from when a same-boot process loss meant the block had
   *  stopped. Closing or quitting the desktop app no longer sets it (the
   *  engine owns the session); historical records still show it. */
  force_quit?: boolean;
  /** Step 11 anti-tamper: true once any extension has applied this
   *  session's policy (persisted, so a resumed session keeps its
   *  transient-disconnect status instead of failing closed). */
  policy_applied?: boolean;
  /** Step 11 anti-tamper: live fail-closed flag — the session
   *  requires website blocking but its policy was never applied,
   *  so the browser app itself is blocked while it runs. */
  browser_fail_closed?: boolean;
  /** Step 14 grace period: wall-clock instant (epoch ms) until
   *  which this auto-started session does not enforce yet.
   *  Absent for manual sessions, when grace is off, and after
   *  grace ends. The Dashboard shows a countdown from this. */
  grace_until_ms?: number | null;
  /** Schedule that auto-created this session, if any. */
  schedule_id?: string | null;
  /** How this session was created: instant focus session,
   *  scheduled block, or one-shot timer. Recorded by the backend
   *  at creation and persisted, so a one-shot session keeps its
   *  kind after its timer record is purged. Absent for sessions
   *  saved before the backend recorded it — the label falls back
   *  to `schedule_id` for those. */
  kind?: 'focus' | 'scheduled' | 'one_shot';
  /** Live Pomodoro phase state (step 7); absent for pre-step saves. */
  pomodoro?: PomodoroState;
  /** Live reward-break state (step 9); absent for pre-step saves. */
  reward?: RewardState;
  /** Lock configuration (step 10); absent for pre-step saves. */
  lock?: LockConfig;
  /** Live lock state (step 10): the challenge to type and any
   *  pending delay request. */
  lock_state?: LockState;
  focus?: FocusConfig;
  report?: FocusReport;
  attention?: AttentionTracking;
}
export interface Task {
  id: string;
  title: string;
  completed: boolean;
  created_at: number;
}
export interface SavedSetup {
  id: string;
  name: string;
  minutes: number;
  focus: FocusConfig;
}
export interface Preferences {
  theme: 'dark' | 'light' | 'system';
  default_minutes: number;
  large_text?: boolean;
  menu_timer?: boolean;
  daily_goal?: number;
  session_alert?: boolean;
  sound?: boolean;
  session_start_alert?: boolean;
  session_start_sound?: boolean;
  volume?: number;
  reminder?: boolean;
  reminder_time?: string;
  reminder_days?: number[];
  offer_break?: boolean;
  break_minutes?: number;
  break_alert?: boolean;
  notifications_enabled?: boolean;
  notifications_sound?: boolean;
  /** Step 14: comma-separated minutes before a scheduled or
   *  one-shot block starts, e.g. '2' or '5, 15'. Empty means no warnings. */
  warning_minutes?: string;
  /** Step 14: grace seconds after a scheduled or one-shot block
   *  starts before blocking begins (0 = off, max 300). */
  grace_seconds?: number;
  saved_setups?: SavedSetup[];
  /** Screen Time category corrections, keyed by app bundle id/path or
   *  website domain (see categoryKey in services/appCategories).
   *  Checked before the built-in mapping everywhere the Screen Time
   *  page categorizes usage, so a correction re-colors past days too. */
  category_overrides?: Record<string, 'Distracting' | 'Productive' | 'Others'>;
}
/** Recurring weekly block (step 5). `focus` is a full snapshot
 *  copied from the template session at creation time: editing the
 *  template later never changes the schedule. Days use 0 = Sunday ..
 *  6 = Saturday; an end_time earlier than start_time is an overnight
 *  window crossing midnight. */
export interface Schedule {
  id: string;
  title: string;
  days: number[];
  start_time: string;
  end_time: string;
  focus: FocusConfig;
  enabled: boolean;
  lock_changes: boolean;
  /** Step 10 lock, in addition to `lock_changes`; absent for
   *  pre-step saves. */
  lock?: LockConfig;
  /** Live step 10 lock state (challenge, pending delay). */
  lock_state?: LockState;
  created_at: number;
}
/** One-shot start timer (step 6): one future start, no recurrence.
 *  `focus` is a full snapshot copied from the template session at
 *  creation time, and `allow_pause` is copied from it too. Once
 *  started, the session is an ordinary duration-bound session.
 *  `fire_at_ms` is a wall-clock epoch timestamp in milliseconds. */
export interface OneShot {
  id: string;
  title: string;
  minutes: number;
  allow_pause: boolean;
  focus: FocusConfig;
  fire_at_ms: number;
  created_at: number;
}
/** Time allowance (step 8): a daily/weekly budget for one site or
 *  app. `usage` is the backend's per-minute bucket log; remaining
 *  time and exhaustion are computed from it (see
 *  src/services/allowances.ts), never stored, so a period roll,
 *  disable, or delete lifts the block. Budgets deplete only during
 *  a Running session's Focus phases and are enforced only while a
 *  session runs. */
export interface AllowanceUsageBucket {
  minute_ms: number;
  ms: number;
}
export interface Allowance {
  id: string;
  title: string;
  kind: 'site' | 'app';
  /** Site: a plain or wildcard domain rule (no search: keywords,
   *  no paths). App: a platform app id. */
  target: string;
  budget_minutes: number;
  period: 'day' | 'week';
  mode: 'rolling' | 'fixed';
  enabled: boolean;
  usage?: AllowanceUsageBucket[];
  created_at: number;
}
/** App lock: an independent timed or daily-limit block for one site
 *  or app, enforced whether or not a focus session runs. */
export interface AppLock {
  id: string;
  title: string;
  kind: 'site' | 'app';
  /** Site: a plain or wildcard domain rule (no search: keywords,
   *  no paths). App: a platform app id. */
  target: string;
  rule: AppLockRule;
  enabled: boolean;
  created_at: number;
}
/** How an app lock blocks, as stored in the snapshot. */
export type AppLockRule =
  | { type: 'timed'; ends_at_ms: number }
  | { type: 'daily_limit'; limit_ms: number };
/** Frontend input for `create_app_lock`: whole minutes, so the
 *  action JSON stays small; the backend computes the stored rule. */
export type AppLockRuleInput =
  | { kind: 'timed'; minutes: number }
  | { kind: 'daily_limit'; limit_minutes: number };
export interface ScreenTimeBucket {
  day_ms: number;
  kind: 'app' | 'site';
  target: string;
  name: string;
  ms: number;
}
export interface StrictBlockState {
  enabled: boolean;
  strict: boolean;
  ends_at_ms?: number | null;
  activating_until_ms?: number | null;
  created_at_ms: number;
}
export interface AdultBlock {
  enabled: boolean;
  strict: boolean;
  ends_at_ms?: number | null;
  activating_until_ms?: number | null;
  created_at_ms: number;
}
export interface Snapshot {
  sessions: Session[];
  tasks: Task[];
  schedules?: Schedule[];
  one_shots?: OneShot[];
  allowances?: Allowance[];
  app_locks?: AppLock[];
  strict_apps?: StrictBlockState;
  strict_sites?: StrictBlockState;
  adult_block?: AdultBlock;
  screen_time_usage?: ScreenTimeBucket[];
  preferences: Preferences;
  browser?: BrowserStatus;
  blocked_app_notice?: BlockedAppNotice | null;
}
/** "Why am I blocked" app page: the last blocked app + its true
 *  reason, shown in the fullscreen `blocked-app` overlay window. */
export interface BlockedAppNotice {
  app_name: string;
  reason: string;
  ends_at_ms?: number | null;
  at_ms: number;
}
export interface Capability {
  id: string;
  name: string;
  available: boolean;
  reason: string;
}
export type Action =
  | { type: 'update_preferences'; preferences: Preferences }
  | {
      type: 'create_session';
      title: string;
      minutes: number;
      allow_pause: boolean;
      focus?: FocusConfig;
      /** Step 10 lock; `lock_password` is hashed by the backend
       *  and never stored. */
      lock?: LockConfig;
      lock_password?: string;
    }
  | {
      type: 'start_session' | 'resume_session' | 'restart_session';
      id: string;
    }
  | {
      type: 'pause_session' | 'end_session' | 'delete_session';
      id: string;
      /** Proofs for a locked session (step 10). */
      password?: string;
      challenge_answer?: string;
    }
  | { type: 'request_lock_delay'; id: string; for_action: string }
  | { type: 'cancel_lock_delay'; id: string }
  | {
      type: 'create_schedule';
      title: string;
      days: number[];
      start_time: string;
      end_time: string;
      focus: FocusConfig;
      enabled: boolean;
      lock_changes: boolean;
      lock?: LockConfig;
      lock_password?: string;
    }
  | {
      type: 'update_schedule';
      id: string;
      title: string;
      days: number[];
      start_time: string;
      end_time: string;
      focus: FocusConfig;
      enabled: boolean;
      lock_changes: boolean;
      /** Absent keeps the existing lock (quick toggle path);
       *  present replaces it wholesale; the frontend sends the
       *  stored hash back while the password lock stays on, and
       *  a new `lock_password` replaces the hash. */
      lock?: LockConfig;
      lock_password?: string;
      password?: string;
      challenge_answer?: string;
    }
  | {
      type: 'delete_schedule';
      id: string;
      password?: string;
      challenge_answer?: string;
    }
  | { type: 'request_schedule_lock_delay'; id: string }
  | { type: 'cancel_schedule_lock_delay'; id: string }
  | {
      type: 'create_one_shot';
      title: string;
      minutes: number;
      allow_pause: boolean;
      focus: FocusConfig;
      fire_at_ms: number;
    }
  | { type: 'delete_one_shot'; id: string }
  | {
      type: 'create_allowance';
      title: string;
      kind: 'site' | 'app';
      target: string;
      budget_minutes: number;
      period: 'day' | 'week';
      mode: 'rolling' | 'fixed';
      enabled: boolean;
    }
  | { type: 'update_allowance'; allowance: Allowance }
  | { type: 'delete_allowance'; id: string }
  | {
      type: 'create_app_lock';
      title: string;
      kind: 'site' | 'app';
      target: string;
      rule: AppLockRuleInput;
    }
  | { type: 'delete_app_lock'; id: string }
  | {
      type: 'start_strict_block';
      target: 'apps' | 'sites' | 'adult';
      days: number;
    }
  | {
      type: 'cancel_strict_block';
      target: 'apps' | 'sites' | 'adult';
    }
  | {
      type: 'set_adult_block';
      enabled: boolean;
      strict: boolean;
      minutes?: number | null;
    }
  | { type: 'take_reward_break' | 'cancel_delayed_break'; id: string }
  | { type: 'disable_camera' | 'enable_camera'; id: string }
  | { type: 'correct_interval'; id: string; interval_id: string; note: string }
  | { type: 'create_task'; title: string }
  | { type: 'update_task'; id: string; title: string; completed: boolean }
  | { type: 'delete_task'; id: string }
  | {
      type: 'set_preferences';
      theme: Preferences['theme'];
      default_minutes: number;
    };

export interface FocusConfig {
  mode: 'standard' | 'strict' | 'deep';
  apps: string[];
  domains: string[];
  /** Per-block exceptions: host/path rules that stay accessible even
   *  when they match a block rule. Blocklist mode only; the backend
   *  strips them in allowlist/offline mode. Never a search: rule. */
  exceptions?: string[];
  /** App-blocking depth (step 4): folders whose apps are blocked
   *  during the session, even if otherwise allowed. System folders
   *  are rejected by the backend validator. */
  blocked_folders?: string[];
  /** App-blocking depth (step 4): window-title substrings. If the
   *  frontmost window's title contains one of these, the app counts
   *  as blocked during the session, even if otherwise allowed. */
  blocked_titles?: string[];
  /** YouTube channel rules (step 3), stored normalized:
   *  `@handle`, a bare UC channel ID, or `c:name` / `user:name`.
   *  Block-list mode blocks the listed channels; study mode allows
   *  ONLY the allowed list. */
  youtube_blocked_channels?: string[];
  youtube_allowed_channels?: string[];
  /** Study mode: only the allowed channels play; the extension also
   *  blocks the YouTube homepage, Shorts, feeds and search results. */
  youtube_study_mode?: boolean;
  /** Entertainment blocking: blocks YouTube entertainment categories
   *  and formats (comedy, pranks, vlogs, funny/animal clips, gaming,
   *  music, Shorts) while keeping non-entertainment videos playable. */
  youtube_block_entertainment?: boolean;
  /** The Watch Later playlist page (youtube.com/playlist?list=WL)
   *  stays open even when YouTube is blocked or not in the allowlist;
   *  every other YouTube route stays blocked. Extension-enforced only. */
  youtube_allow_watch_later?: boolean;
  block_adult_content?: boolean;
  website_blocking?: 'auto' | 'none' | 'blocklist' | 'allowlist';
  general_internet: boolean;
  browser_app: string;
  activity_consent: boolean;
  camera: boolean;
  attention?: AttentionPolicy;
  /** Pomodoro focus/break cycles (step 7); absent means disabled. */
  pomodoro?: PomodoroConfig;
  /** Reward and delay breaks (step 9); absent means disabled. */
  reward?: RewardConfig;
  /** Frozen Turkey (step 12); absent means not frozen. */
  frozen?: FrozenConfig;
}
/** Frozen Turkey configuration (step 12): a full-computer lock.
 *  `allowed_apps` are app ids (bundle ids on macOS, exe paths or
 *  `store:` ids on Windows); empty means only Preelock itself plus
 *  protected system processes. `allowed_websites` are host rules
 *  in the same language as `domains`, never `search:` rules;
 *  empty means no website is allowed. While frozen, the session
 *  has no Pomodoro or reward breaks, cannot be paused or ended
 *  early, and ends only when its wall-clock timer runs out. */
export interface FrozenConfig {
  enabled: boolean;
  allowed_apps?: string[];
  allowed_websites?: string[];
}
/** Display label for a frozen config, or null when not frozen. */
export function frozenLabel(focus: FocusConfig | undefined): string | null {
  if (!focus?.frozen?.enabled) return null;
  const apps = focus.frozen.allowed_apps?.length ?? 0;
  const sites = focus.frozen.allowed_websites?.length ?? 0;
  const parts: string[] = [];
  parts.push(
    apps
      ? `${apps} app${apps === 1 ? '' : 's'} allowed`
      : 'only Preelock + system apps',
  );
  parts.push(
    sites
      ? `${sites} site${sites === 1 ? '' : 's'} allowed`
      : 'no websites allowed',
  );
  return parts.join(' · ');
}
/** Pomodoro configuration (step 7): focus length, break lengths, and
 *  how many focus cycles run before a long break. The backend
 *  validates the ranges (focus 1-180, short 1-60, long 1-90, cycles
 *  2-8); disabled configs are never rejected. */
export interface PomodoroConfig {
  enabled: boolean;
  focus_minutes: number;
  short_break_minutes: number;
  long_break_minutes: number;
  cycles_before_long_break: number;
}
/** Live Pomodoro phase state (step 7), as reported by the backend. */
export interface PomodoroState {
  phase: 'focus' | 'short_break' | 'long_break';
  phase_remaining_ms: number;
  completed_focus_cycles: number;
}
/** Reward-break configuration (step 9): productive focus minutes
 *  that earn break minutes, the credit cap, the longest single
 *  break, and the delay before a requested break starts (0 starts
 *  at once). The backend validates the ranges (earn focus 1-180,
 *  earn break 1-60, max credit 1-120, max break 1-60, delay 0-60);
 *  disabled configs are never rejected. */
export interface RewardConfig {
  enabled: boolean;
  earn_focus_minutes: number;
  earn_break_minutes: number;
  max_credit_minutes: number;
  max_break_minutes: number;
  delay_minutes: number;
}
/** Live reward-break state (step 9), as reported by the backend.
 *  `pending_delay_ms` > 0 means a delayed break is waiting (focus
 *  continues); `break_remaining_ms` > 0 means a reward break is
 *  running (blocking relaxed). Never both at once. */
export interface RewardState {
  credit_ms: number;
  earn_progress_ms: number;
  pending_delay_ms: number;
  break_remaining_ms: number;
}
/** Lock configuration (step 10). `password_hash` is set by the
 *  backend only (from a one-time `lock_password` on create/edit)
 *  and is never sent by the frontend; the frontend sends a lock
 *  with no hash and a separate password field. `challenge_length`
 *  0 means no challenge, `delay_minutes` 0 means no delay. */
export interface LockConfig {
  password_hash?: string | null;
  challenge_length?: number;
  delay_minutes?: number;
  restart_lock?: boolean;
}
/** Live lock state (step 10), as reported by the backend. */
export interface LockState {
  challenge?: string | null;
  delay_until_ms?: number | null;
  delay_for?: string | null;
}
export const defaultLock: LockConfig = {
  challenge_length: 0,
  delay_minutes: 0,
  restart_lock: false,
};
/** True when the lock can actually block an action (a
 *  restart-only lock is persistence, not a gate). */
export function lockHasGate(lock: LockConfig | undefined): boolean {
  if (!lock) return false;
  return (
    !!lock.password_hash ||
    (lock.challenge_length ?? 0) > 0 ||
    (lock.delay_minutes ?? 0) > 0
  );
}
/** Display label for a lock, or null when nothing is locked. */
export function lockLabel(lock: LockConfig | undefined): string | null {
  if (!lock) return null;
  const parts: string[] = [];
  if (lock.password_hash) parts.push('password');
  if ((lock.challenge_length ?? 0) > 0)
    parts.push(`challenge (${lock.challenge_length} chars)`);
  if ((lock.delay_minutes ?? 0) > 0)
    parts.push(`delay ${lock.delay_minutes} min`);
  if (lock.restart_lock) parts.push('survives restart');
  return parts.length ? parts.join(' · ') : null;
}
/** Delay status for a lock state, computed in one place so
 *  components never call `Date.now()` during render. */
export function lockDelayStatus(state: LockState | undefined): {
  kind: 'pending' | 'done' | 'none';
  minutesLeft: number;
} {
  if (!state?.delay_for || state.delay_until_ms == null)
    return { kind: 'none', minutesLeft: 0 };
  const ms = state.delay_until_ms - Date.now();
  return ms > 0
    ? { kind: 'pending', minutesLeft: Math.ceil(ms / 60000) }
    : { kind: 'done', minutesLeft: 0 };
}
/** Display label for a Pomodoro phase; null when there is no state. */
export function pomodoroLabel(state: PomodoroState | undefined): string | null {
  if (!state) return null;
  return state.phase === 'focus'
    ? 'Focus'
    : state.phase === 'short_break'
      ? 'Short break'
      : 'Long break';
}

/** Step 11 tamper visibility: concise one-line note for the latest
 *  tamper event in a session's report, or null when there is none.
 *  Used by SessionCard (Dashboard + Activity); the full report
 *  detail is never shown in the card, and non-tamper events
 *  (focus checks, browser disconnects) never produce a note. */
export function tamperNote(session: Session): string | null {
  const tamper = [...(session.report?.events ?? [])]
    .reverse()
    .find((ev) => ev.kind === 'clock_jump');
  if (!tamper) return null;
  return 'Clock jump noted — jumped time was not credited.';
}
export interface FocusReport {
  events: { at: number; kind: string; detail: string }[];
  approved_ms: number;
  distracting_ms: number;
  idle_ms: number;
  interruptions: number;
  active_app: string;
  enforcement: string;
  camera_status: string;
  browser_blocks: number;
}
/** Step 13: one recorded block attempt (SQLite, app attempts only;
 *  website blocking has no per-domain rows — see stats.rs). */
export interface BlockAttempt {
  at_ms: number;
  kind: 'app' | 'site';
  target: string;
  session_id: string | null;
  reason: string;
}
export interface BrowserStatus {
  available: boolean;
  connected: boolean;
  applied_id: string;
  client_browser?: string;
  search_distracted?: boolean;
  channel_blocked?: boolean;
  error: string;
  blocks: number;
  cleanup_pending: boolean;
  /** The connected extension copy predates the behavior the app shipped with. */
  extension_stale: boolean;
}
/**
 * True when `id` — a platform browser id (macOS bundle id, or a Windows
 * executable path) — refers to the browser the extension reported in its
 * sync (`reported`, a canonical bundle id). Mirrors `same_browser` in the
 * Rust bridge. Empty values never match.
 */
export function sameBrowser(id: string, reported: string): boolean {
  const a = (id || '').toLowerCase();
  const b = (reported || '').toLowerCase();
  if (!a || !b) return false;
  if (a === b) return true;
  const exe: Record<string, string> = {
    'com.google.chrome': 'chrome.exe',
    'com.microsoft.edgemac': 'msedge.exe',
    'com.brave.browser': 'brave.exe',
    'com.vivaldi.vivaldi': 'vivaldi.exe',
    'org.chromium.chromium': 'chromium.exe',
    'ai.perplexity.comet': 'comet.exe',
  };
  const name = exe[b];
  return !!name && a.endsWith('\\' + name);
}
/** The single browser a session's website rules cover: the chosen browser.
 *  One browser per session. Mirrors `FocusConfig::session_browsers` in the
 *  Rust backend. */
export function sessionBrowsers(focus: FocusConfig): string[] {
  return focus.browser_app ? [focus.browser_app] : [];
}
export interface FocusSetup {
  browsers?: { id: string; name: string }[];
  extension_directory?: string;
  /** Browser ids with a live extension connection. */
  connected_browsers?: string[];
  platform: string;
  native_available: boolean;
  applications: { id: string; name: string }[];
  camera_permission: string;
  browser: BrowserStatus;
  pairing_token: string;
  default_browser?: string;
}
export const defaultFocus: FocusConfig = {
  mode: 'standard',
  apps: [],
  domains: [],
  exceptions: [],
  blocked_folders: [],
  blocked_titles: [],
  youtube_blocked_channels: [],
  youtube_allowed_channels: [],
  youtube_study_mode: false,
  youtube_block_entertainment: false,
  youtube_allow_watch_later: false,
  block_adult_content: false,
  website_blocking: 'auto',
  general_internet: true,
  browser_app: '',
  activity_consent: true,
  camera: false,
  attention: {
    enabled: true,
    task: 'other',
    verification: 'activity_only',
    grace_seconds: 10,
    camera_fallback: 'unknown',
    work_apps: [],
    work_domains: [],
  },
  pomodoro: {
    enabled: false,
    focus_minutes: 25,
    short_break_minutes: 5,
    long_break_minutes: 15,
    cycles_before_long_break: 4,
  },
  reward: {
    enabled: false,
    earn_focus_minutes: 25,
    earn_break_minutes: 5,
    max_credit_minutes: 30,
    max_break_minutes: 30,
    delay_minutes: 0,
  },
  frozen: {
    enabled: false,
    allowed_apps: [],
    allowed_websites: [],
  },
};

export interface AttentionPolicy {
  enabled: boolean;
  task: string;
  verification: 'activity_only' | 'camera_and_activity';
  grace_seconds: number;
  camera_fallback: 'unknown' | 'activity_only';
  work_apps: string[];
  work_domains: string[];
}
export interface AttentionTracking {
  state: string;
  reason: string;
  elapsed_ms: number;
  distracted_ms: number;
  absent_ms: number;
  unknown_ms: number;
  corrected_ms: number;
  intervals: {
    id: string;
    at: number;
    duration_ms: number;
    state: string;
    reason: string;
    corrected_ms: number;
    correction_note: string;
  }[];
}
