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
  app_locks: [],
  strict_apps: {
    enabled: false,
    strict: false,
    ends_at_ms: null,
    activating_until_ms: null,
    created_at_ms: 0,
  },
  strict_sites: {
    enabled: false,
    strict: false,
    ends_at_ms: null,
    activating_until_ms: null,
    created_at_ms: 0,
  },
  adult_block: {
    enabled: false,
    strict: false,
    ends_at_ms: null,
    activating_until_ms: null,
    created_at_ms: 0,
  },
  preferences: { theme: 'dark', default_minutes: 25, paused: false },
  unconnected_browser: null,
};
// A backend call must never hang the UI forever. If the Rust side stalls
// (e.g. waiting on the engine), reject after `ms` so the store's `busy`
// flag clears and the buttons unfreeze instead of staying shaded.
// (2026-11-23 fix: Start block froze the whole App lock section.)
function withTimeout<T>(
  promise: Promise<T>,
  ms: number,
  label: string,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(
      () => reject(new Error(`${label} timed out after ${ms / 1000}s`)),
      ms,
    );
  });
  return Promise.race([promise, timeout]).finally(() => {
    if (timer) clearTimeout(timer);
  });
}
export const api = {
  openSetup: (
    target:
      | 'install_extension'
      | 'extensions'
      | 'extension_folder'
      | 'camera_settings'
      | 'notification_settings'
      | 'privacy_settings'
      | 'accessibility_settings'
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
    desktop
      ? withTimeout(invoke<Snapshot>('snapshot'), 10_000, 'Snapshot')
      : Promise.resolve(emptySnapshot),
  execute: (action: Action) =>
    desktop
      ? withTimeout(invoke<Snapshot>('execute', { action }), 20_000, 'Save')
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
  getDnsConsent: () =>
    desktop ? invoke<boolean | null>('get_dns_consent') : Promise.resolve(null),
  setDnsConsent: (granted: boolean) =>
    desktop ? invoke<void>('set_dns_consent', { granted }) : Promise.resolve(),
  dnsProxyStatus: () =>
    desktop
      ? invoke<Record<string, unknown>>('dns_proxy_status')
      : Promise.resolve({}),
  isNativeBlockingSetupRequired: () =>
    desktop
      ? invoke<boolean>('is_native_blocking_setup_required')
      : Promise.resolve(false),
  setupNativeBlocking: () =>
    desktop ? invoke<void>('setup_native_blocking') : Promise.resolve(),
  isPrivilegedHelperInstalled: () =>
    desktop
      ? invoke<boolean>('is_privileged_helper_installed')
      : Promise.resolve(false),
  installPrivilegedHelper: () =>
    desktop ? invoke<void>('install_privileged_helper') : Promise.resolve(),
  uninstallPrivilegedHelper: () =>
    desktop ? invoke<void>('uninstall_privileged_helper') : Promise.resolve(),
  getSystemBlockingStatus: () =>
    desktop
      ? invoke<{
          hosts_entries: number;
          pf_rules: number;
          helper_installed: boolean;
          helper_domains: string[];
          strict_until: number | null;
          clean: boolean;
        }>('get_system_blocking_status')
      : Promise.resolve({
          hosts_entries: 0,
          pf_rules: 0,
          helper_installed: false,
          helper_domains: [],
          strict_until: null,
          clean: true,
        }),
  flushSystemBlocks: () =>
    desktop ? invoke<void>('flush_system_blocks') : Promise.resolve(),
  restartBackgroundEngine: () =>
    desktop ? invoke<void>('restart_background_engine') : Promise.resolve(),
};
