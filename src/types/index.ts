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
  /** True when the app was killed mid-session (force quit, crash). */
  force_quit?: boolean;
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
  saved_setups?: SavedSetup[];
}
export interface Snapshot {
  sessions: Session[];
  tasks: Task[];
  preferences: Preferences;
  browser?: BrowserStatus;
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
    }
  | {
      type:
        | 'start_session'
        | 'pause_session'
        | 'resume_session'
        | 'end_session'
        | 'delete_session'
        | 'restart_session';
      id: string;
    }
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
  website_blocking?: 'auto' | 'none' | 'blocklist' | 'allowlist';
  general_internet: boolean;
  browser_app: string;
  activity_consent: boolean;
  camera: boolean;
  attention?: AttentionPolicy;
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
export interface BrowserStatus {
  available: boolean;
  connected: boolean;
  applied_id: string;
  client_browser?: string;
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
}
export const defaultFocus: FocusConfig = {
  mode: 'standard',
  apps: [],
  domains: [],
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
