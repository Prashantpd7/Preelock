export type SessionStatus =
  'ready' | 'running' | 'paused' | 'completed' | 'ended';
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
    }
  | {
      type:
        'start_session' | 'pause_session' | 'resume_session' | 'end_session';
      id: string;
    }
  | { type: 'create_task'; title: string }
  | { type: 'update_task'; id: string; title: string; completed: boolean }
  | {
      type: 'set_preferences';
      theme: Preferences['theme'];
      default_minutes: number;
    };
