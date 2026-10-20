import { invoke, isTauri } from '@tauri-apps/api/core';
import type { Action, Capability, Snapshot, FocusSetup } from '../types';
export const desktop = isTauri();
export const emptySnapshot: Snapshot = {
  sessions: [],
  tasks: [],
  preferences: { theme: 'dark', default_minutes: 25 },
};
export const api = {
  openSetup: (
    target:
      | 'extensions'
      | 'extension_folder'
      | 'camera_settings'
      | 'notification_settings'
      | 'privacy_settings',
    browserId = '',
  ) => invoke<void>('open_setup', { target, browserId }),
  focusSetup: (browserId?: string) =>
    invoke<FocusSetup>('focus_setup', { browserId: browserId ?? null }),
  requestCamera: () => invoke<void>('request_camera'),
  resetPairing: () => invoke<void>('reset_browser_pairing'),
  snapshot: () =>
    desktop ? invoke<Snapshot>('snapshot') : Promise.resolve(emptySnapshot),
  execute: (action: Action) =>
    desktop
      ? invoke<Snapshot>('execute', { action })
      : Promise.reject(
          new Error('Open the desktop app to save tasks and sessions.'),
        ),
  capabilities: () =>
    desktop
      ? invoke<Capability[]>('capabilities')
      : Promise.resolve<Capability[]>([]),
};
