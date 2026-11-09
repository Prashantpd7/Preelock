import { describe, expect, it } from 'vitest';
import {
  appLockStatus,
  appLockTodayUsageMs,
  formatAppLockMinutes,
  normalizeAppLockDomain,
  recentApps,
  validateAppLockForm,
} from '../services/appLocks';
import type { AppLock, ScreenTimeBucket } from '../types';

const DAY = 1_799_990_400_000;
const NOW = 1_800_000_000_000;

function bucket(
  kind: 'app' | 'site',
  target: string,
  ms: number,
  dayMs = DAY,
): ScreenTimeBucket {
  return { day_ms: dayMs, kind, target, name: target, ms };
}

function appTimedLock(overrides: Partial<AppLock> = {}): AppLock {
  return {
    id: 'l1',
    title: 'Game',
    kind: 'app',
    target: 'com.example.game',
    rule: { type: 'timed', ends_at_ms: NOW + 133 * 60_000 },
    enabled: true,
    created_at: NOW - 60_000,
    ...overrides,
  };
}

function siteDailyLock(overrides: Partial<AppLock> = {}): AppLock {
  return {
    id: 'l2',
    title: 'youtube.com',
    kind: 'site',
    target: 'youtube.com',
    rule: { type: 'daily_limit', limit_ms: 120 * 60_000 },
    enabled: true,
    created_at: NOW - 60_000,
    ...overrides,
  };
}

describe('normalizeAppLockDomain', () => {
  it('strips the scheme, path, query and fragment, and lowercases', () => {
    expect(normalizeAppLockDomain('https://YouTube.com/watch?v=1 ')).toBe(
      'youtube.com',
    );
    expect(normalizeAppLockDomain('http://example.com/#frag')).toBe(
      'example.com',
    );
  });
  it('keeps www. as typed', () => {
    expect(normalizeAppLockDomain('https://www.example.com/')).toBe(
      'www.example.com',
    );
  });
  it('keeps wildcards as typed', () => {
    expect(normalizeAppLockDomain('*.Example.com')).toBe('*.example.com');
  });
});

describe('validateAppLockForm', () => {
  const valid = {
    title: 'YouTube',
    kind: 'site' as const,
    target: 'youtube.com',
    mode: 'timed' as const,
    minutes: 60,
  };
  it('accepts a valid form', () => {
    expect(validateAppLockForm(valid)).toBe('');
  });
  it('requires a target', () => {
    expect(validateAppLockForm({ ...valid, target: '  ' })).toBe(
      'Enter a website domain to block.',
    );
    expect(validateAppLockForm({ ...valid, kind: 'app', target: '' })).toBe(
      'Pick an app to block.',
    );
  });
  it('rejects search: keywords and paths for sites', () => {
    expect(validateAppLockForm({ ...valid, target: 'search:chess' })).toBe(
      'An app lock needs a domain, not a search: keyword — keywords have no host to watch.',
    );
    expect(
      validateAppLockForm({ ...valid, target: 'youtube.com/shorts' }),
    ).toBe(
      'An app lock needs a plain or wildcard domain, not a domain with a path.',
    );
  });
  it('requires a title of 1-160 characters', () => {
    expect(validateAppLockForm({ ...valid, title: '   ' })).toBe(
      'Use a title between 1 and 160 characters.',
    );
  });
  it('range-checks timed minutes 1-10080', () => {
    expect(validateAppLockForm({ ...valid, minutes: 0 })).toBe(
      'Choose a timed lock duration from 1 to 10080 minutes (7 days).',
    );
    expect(validateAppLockForm({ ...valid, minutes: 10081 })).toBe(
      'Choose a timed lock duration from 1 to 10080 minutes (7 days).',
    );
    expect(validateAppLockForm({ ...valid, minutes: 10080 })).toBe('');
  });
  it('range-checks daily-limit minutes 1-1440', () => {
    const daily = { ...valid, mode: 'daily' as const };
    expect(validateAppLockForm({ ...daily, minutes: 1441 })).toBe(
      'Choose a daily limit from 1 to 1440 minutes.',
    );
    expect(validateAppLockForm({ ...daily, minutes: 1440 })).toBe('');
  });
});

describe('recentApps', () => {
  it('groups app buckets by target, sums usage, sorts desc, ignores sites', () => {
    const apps = recentApps([
      bucket('app', 'com.example.a', 10 * 60_000),
      bucket('site', 'youtube.com', 999 * 60_000),
      bucket('app', 'com.example.b', 30 * 60_000),
      bucket('app', 'com.example.a', 5 * 60_000),
    ]);
    expect(apps.map((a) => a.target)).toEqual([
      'com.example.b',
      'com.example.a',
    ]);
    expect(apps[1].ms).toBe(15 * 60_000);
  });
});

describe('appLockTodayUsageMs', () => {
  it('sums only today buckets for the target', () => {
    const buckets = [
      bucket('app', 'com.example.game', 20 * 60_000),
      bucket('app', 'com.example.game', 10 * 60_000, DAY - 86_400_000),
      bucket('app', 'com.example.other', 50 * 60_000),
    ];
    expect(appLockTodayUsageMs(buckets, DAY, 'app', 'com.example.game')).toBe(
      20 * 60_000,
    );
  });
  it('matches a plain domain with itself and subdomains', () => {
    const buckets = [
      bucket('site', 'youtube.com', 10 * 60_000),
      bucket('site', 'www.youtube.com', 15 * 60_000),
      bucket('site', 'notyoutube.com', 99 * 60_000),
    ];
    expect(appLockTodayUsageMs(buckets, DAY, 'site', 'youtube.com')).toBe(
      25 * 60_000,
    );
  });
  it('matches a wildcard rule with subdomains only', () => {
    const buckets = [
      bucket('site', 'app.example.com', 10 * 60_000),
      bucket('site', 'example.com', 99 * 60_000),
    ];
    expect(appLockTodayUsageMs(buckets, DAY, 'site', '*.example.com')).toBe(
      10 * 60_000,
    );
  });
});

describe('formatAppLockMinutes', () => {
  it('formats minutes and hours', () => {
    expect(formatAppLockMinutes(45 * 60_000)).toBe('45m');
    expect(formatAppLockMinutes(2 * 3_600_000)).toBe('2h');
    expect(formatAppLockMinutes(133 * 60_000)).toBe('2h 13m');
  });
});

describe('appLockStatus', () => {
  it('shows remaining time for a timed lock', () => {
    expect(appLockStatus(appTimedLock(), [], DAY, NOW)).toEqual({
      enforced: true,
      text: '2h 13m left',
    });
  });
  it('shows Expired once the end has passed', () => {
    const lock = appTimedLock({
      rule: { type: 'timed', ends_at_ms: NOW - 1000 },
    });
    expect(appLockStatus(lock, [], DAY, NOW)).toEqual({
      enforced: false,
      text: 'Expired',
    });
  });
  it('shows used vs limit for a daily lock under the limit', () => {
    const buckets = [bucket('site', 'www.youtube.com', 45 * 60_000)];
    expect(appLockStatus(siteDailyLock(), buckets, DAY, NOW)).toEqual({
      enforced: false,
      text: '45m of 2h used today',
    });
  });
  it('shows blocking once usage reaches the limit', () => {
    const buckets = [bucket('site', 'youtube.com', 120 * 60_000)];
    expect(appLockStatus(siteDailyLock(), buckets, DAY, NOW)).toEqual({
      enforced: true,
      text: 'Limit reached — blocking until midnight',
    });
  });
});
