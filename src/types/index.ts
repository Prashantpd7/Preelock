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
export interface Preferences {
  theme: 'dark' | 'light' | 'system';
  default_minutes: number;
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
  | {
      type: 'create_session';
      title: string;
      minutes: number;
      allow_pause: boolean;
      focus?: FocusConfig;
    }
  | {
      type:
        'start_session' | 'pause_session' | 'resume_session' | 'end_session';
      id: string;
    }
  | { type: 'emergency_exit'; id: string; confirmation: string }
  | { type: 'disable_camera' | 'confirm_work'; id: string }
  | { type: 'correct_interval'; id: string; interval_id: string; note: string }
  | { type: 'create_task'; title: string }
  | { type: 'update_task'; id: string; title: string; completed: boolean }
  | {
      type: 'set_preferences';
      theme: Preferences['theme'];
      default_minutes: number;
    };

export interface FocusConfig {
  mode: 'standard' | 'strict' | 'deep';
  apps: string[];
  domains: string[];
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
  error: string;
  blocks: number;
  cleanup_pending: boolean;
}
export interface FocusSetup {
  browsers?: { id: string; name: string }[];
  extension_directory?: string;
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
  general_internet: true,
  browser_app: '',
  activity_consent: false,
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
  verification: 'activity_only' | 'camera_and_activity' | 'manual_checkpoints';
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
