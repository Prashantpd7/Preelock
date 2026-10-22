import { invoke } from '@tauri-apps/api/core';
import type { Preferences } from '../types';
export const preferenceDefaults: Required<Preferences> = {
  theme: 'dark',
  default_minutes: 25,
  large_text: false,
  menu_timer: false,
  daily_goal: 0,
  session_alert: true,
  sound: false,
  session_start_alert: false,
  session_start_sound: false,
  volume: 50,
  reminder: false,
  reminder_time: '09:00',
  reminder_days: [1, 2, 3, 4, 5],
  offer_break: true,
  break_minutes: 5,
  break_alert: true,
  notifications_enabled: true,
  saved_setups: [],
};
export type SettingsAction =
  | {
      type:
        | 'info'
        | 'status'
        | 'allow_notifications'
        | 'test_browser'
        | 'test_camera'
        | 'stop_camera_test'
        | 'start_break'
        | 'end_break'
        | 'dismiss_break'
        | 'export_history'
        | 'backup'
        | 'reset_preferences';
    }
  | { type: 'login'; enabled: boolean }
  | { type: 'preview_sound'; volume: number }
  | { type: 'restore' | 'delete_history'; confirmation: string };
export interface SettingsInfo {
  platform?: string;
  login?: number;
  notifications?: number;
  version?: string;
  break_remaining?: number;
  break_offer?: string;
  camera_status?: string;
  browser_test?: string;
  cancelled?: boolean;
}
export const settingsAction = (action: SettingsAction) =>
  invoke<SettingsInfo>('settings_action', { action });
