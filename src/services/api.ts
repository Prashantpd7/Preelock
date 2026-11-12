import { invoke, isTauri } from '@tauri-apps/api/core';
import type {
  Action,
  BlockAttempt,
  Capability,
  Snapshot,
  FocusSetup,
} from '../types';
export const desktop = isTauri();
export const emptySnapshot: Snapshot = {
  sessions: [],
  tasks: [],
  schedules: [],
  one_shots: [],
  allowances: [],
  preferences: { theme: 'dark', default_minutes: 25 },
};
export const api = {
  openSetup: (
    target:
      | 'extensions'
      | 'extension_folder'
      | 'camera_settings'
      | 'notification_settings'
      | 'privacy_settings'
      | 'browser_download'
      | 'browser',
    browserId = '',
  ) => invoke<void>('open_setup', { target, browserId }),
  focusSetup: (browserId?: string) =>
    invoke<FocusSetup>('focus_setup', { browserId: browserId ?? null }),
  requestCamera: () => invoke<void>('request_camera'),
  resetPairing: () => invoke<void>('reset_browser_pairing'),
  reconnectBrowser: () =>
    desktop
      ? invoke<void>('settings_action', {
          action: { type: 'reconnect_browser' },
        })
      : Promise.resolve(),
  snapshot: () =>
    desktop ? invoke<Snapshot>('snapshot') : Promise.resolve(emptySnapshot),
  execute: (action: Action) =>
    desktop
      ? invoke<Snapshot>('execute', { action })
      : Promise.reject(
          new Error('Open the desktop app to save tasks and sessions.'),
        ),
  blockAttempts: () =>
    desktop
      ? invoke<BlockAttempt[]>('block_attempts')
      : Promise.resolve<BlockAttempt[]>([]),
  appIcon: (identity: string) =>
    desktop
      ? invoke<string | null>('get_app_icon', { identity })
      : Promise.resolve(null),
  exportStatistics: () => invoke<string>('export_statistics'),
  deleteStatistics: () => invoke<number>('delete_statistics'),
  capabilities: () =>
    desktop
      ? invoke<Capability[]>('capabilities')
      : Promise.resolve<Capability[]>([]),
  dismissBlockedAppPage: () =>
    desktop ? invoke<void>('dismiss_blocked_app_page') : Promise.resolve(),
};
