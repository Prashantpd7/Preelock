import type { AppLock, ScreenTimeBucket } from '../types';

// App locks: independent timed and daily-limit blocks for one site or
// app, enforced whether or not a session runs. These helpers mirror
// the Rust backend in src-tauri/src/applocks.rs, so the status text
// shown here matches what the engine enforces — the backend is always
// authoritative, this is display only.

/** Whole-minute preset durations offered as chips. */
export interface AppLockPreset {
  minutes: number;
  label: string;
}

export const TIMED_PRESETS: AppLockPreset[] = [
  { minutes: 60, label: '1 hour' },
  { minutes: 300, label: '5 hours' },
  { minutes: 600, label: '10 hours' },
  { minutes: 1440, label: '24 hours' },
];

export const LIMIT_PRESETS: AppLockPreset[] = [
  { minutes: 30, label: '30 min' },
  { minutes: 60, label: '1 hour' },
  { minutes: 120, label: '2 hours' },
  { minutes: 180, label: '3 hours' },
];

/** Backend range limits, in whole minutes. */
export const TIMED_MAX_MINUTES = 10080;
export const LIMIT_MAX_MINUTES = 1440;

/** Normalize a website target the way the form promises: strip the
 *  scheme, cut any path/query/fragment, trim whitespace, lowercase.
 *  `www.` is kept as typed (it is part of the host); the backend
 *  still validates strictly and rejects anything it cannot use. */
export function normalizeAppLockDomain(raw: string): string {
  let s = raw.trim().toLowerCase();
  s = s.replace(/^[a-z][a-z0-9+.-]*:\/\//, '');
  s = s.split(/[/?#]/, 1)[0];
  return s.trim();
}

/** Client-side form validation, mirroring the backend rules in
 *  src-tauri/src/applocks.rs (the messages match the backend's, so a
 *  rejection reads the same either way). Empty string means valid. */
export function validateAppLockForm(input: {
  title: string;
  kind: 'site' | 'app';
  target: string;
  mode: 'timed' | 'daily';
  minutes: number;
}): string {
  const target = input.target.trim();
  if (!target) {
    return input.kind === 'site'
      ? 'Enter a website domain to block.'
      : 'Pick an app to block.';
  }
  if (input.kind === 'site') {
    if (target.toLowerCase().startsWith('search:'))
      return 'An app lock needs a domain, not a search: keyword — keywords have no host to watch.';
    if (target.includes('/'))
      return 'An app lock needs a plain or wildcard domain, not a domain with a path.';
  } else if (
    target.length > 1024 ||
    [...target].some((c) => c.charCodeAt(0) < 32 || c.charCodeAt(0) === 127)
  ) {
    return 'Use an app id up to 1024 characters.';
  }
  const title = input.title.trim();
  if (!title || title.length > 160)
    return 'Use a title between 1 and 160 characters.';
  if (input.mode === 'timed') {
    if (
      !Number.isInteger(input.minutes) ||
      input.minutes < 1 ||
      input.minutes > TIMED_MAX_MINUTES
    )
      return `Choose a timed lock duration from 1 to ${TIMED_MAX_MINUTES} minutes (7 days).`;
  } else if (
    !Number.isInteger(input.minutes) ||
    input.minutes < 1 ||
    input.minutes > LIMIT_MAX_MINUTES
  ) {
    return `Choose a daily limit from 1 to ${LIMIT_MAX_MINUTES} minutes.`;
  }
  return '';
}

/** One recently-used app for the picker. */
export interface RecentApp {
  target: string;
  name: string;
  ms: number;
}

/** Recently-used apps from the Screen Time buckets: kind:'app'
 *  buckets grouped by target, usage summed, sorted by usage desc. */
export function recentApps(buckets: ScreenTimeBucket[]): RecentApp[] {
  const grouped = new Map<string, RecentApp>();
  for (const bucket of buckets) {
    if (bucket.kind !== 'app') continue;
    const previous = grouped.get(bucket.target);
    if (previous) previous.ms += bucket.ms;
    else
      grouped.set(bucket.target, {
        target: bucket.target,
        name: bucket.name || bucket.target,
        ms: bucket.ms,
      });
  }
  return [...grouped.values()].sort((a, b) => b.ms - a.ms);
}

/** Whether a site bucket's host counts toward a lock target: a plain
 *  domain matches itself and subdomains, a `*.` rule matches
 *  subdomains only. Mirrors the engine's host matching for display. */
function siteTargetMatches(lockTarget: string, bucketTarget: string): boolean {
  const lock = lockTarget.toLowerCase();
  const bucket = bucketTarget.toLowerCase();
  if (lock.startsWith('*.')) {
    const domain = lock.slice(2);
    return domain.length > 0 && bucket.endsWith(`.${domain}`);
  }
  return bucket === lock || bucket.endsWith(`.${lock}`);
}

/** Today's usage of one lock target from the Screen Time buckets
 *  (display only; mirrors applocks::today_usage_ms). */
export function appLockTodayUsageMs(
  buckets: ScreenTimeBucket[],
  dayMs: number,
  kind: 'site' | 'app',
  target: string,
): number {
  let total = 0;
  for (const bucket of buckets) {
    if (bucket.day_ms !== dayMs || bucket.kind !== kind) continue;
    const matches =
      kind === 'site'
        ? siteTargetMatches(target, bucket.target)
        : bucket.target.toLowerCase() === target.toLowerCase();
    if (matches) total += bucket.ms;
  }
  return total;
}

/** Whole minutes, floored: "45m", "2h", "2h 13m". */
export function formatAppLockMinutes(ms: number): string {
  const minutes = Math.max(0, Math.floor(ms / 60_000));
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest ? `${hours}h ${rest}m` : `${hours}h`;
}

export interface AppLockStatus {
  /** Whether the lock blocks its target right now. */
  enforced: boolean;
  /** One-line live status for the Active blocks row. */
  text: string;
}

/** Live status for one lock, computed from the snapshot (display
 *  only; the backend enforces). `nowMs` comes from the component's
 *  ticking clock — never Date.now() during render. */
export function appLockStatus(
  lock: AppLock,
  buckets: ScreenTimeBucket[],
  todayMs: number,
  nowMs: number,
): AppLockStatus {
  if (lock.rule.type === 'timed') {
    const remainingMs = lock.rule.ends_at_ms - nowMs;
    if (remainingMs <= 0) return { enforced: false, text: 'Expired' };
    return {
      enforced: lock.enabled,
      text: `${formatAppLockMinutes(remainingMs)} left`,
    };
  }
  const usedMs = appLockTodayUsageMs(buckets, todayMs, lock.kind, lock.target);
  const limitMs = lock.rule.limit_ms;
  if (lock.enabled && usedMs >= limitMs)
    return { enforced: true, text: 'Limit reached — blocking until midnight' };
  return {
    enforced: false,
    text: `${formatAppLockMinutes(usedMs)} of ${formatAppLockMinutes(limitMs)} used today`,
  };
}
