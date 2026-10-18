import { invoke, isTauri } from '@tauri-apps/api/core';
import type { Action, Capability, Snapshot } from '../types';
export const desktop = isTauri();
export const emptySnapshot: Snapshot = {
  sessions: [],
  tasks: [],
  preferences: { theme: 'dark', default_minutes: 25 },
};
export const api = {
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
