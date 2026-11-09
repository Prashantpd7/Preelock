import { describe, expect, it } from 'vitest';
import {
  defaultFocus,
  defaultLock,
  lockHasGate,
  lockLabel,
  type LockConfig,
  type Schedule,
} from '../types';
import { isScheduleLockedActive } from '../services/schedules';

const baseSchedule: Schedule = {
  id: 's1',
  title: 'Morning',
  days: [1],
  start_time: '09:00',
  end_time: '10:00',
  focus: defaultFocus,
  enabled: true,
  lock_changes: false,
  created_at: 0,
};

describe('lockLabel', () => {
  it('is null for no lock or an empty lock', () => {
    expect(lockLabel(undefined)).toBeNull();
    expect(lockLabel(defaultLock)).toBeNull();
  });
  it('lists every configured lock', () => {
    const lock: LockConfig = {
      password_hash: 'v1:100000:aa:bb',
      challenge_length: 32,
      delay_minutes: 5,
      restart_lock: true,
    };
    expect(lockLabel(lock)).toBe(
      'password · challenge (32 chars) · delay 5 min · survives restart',
    );
  });
  it('lists a restart-only lock without treating it as a gate', () => {
    const lock: LockConfig = { restart_lock: true };
    expect(lockLabel(lock)).toBe('survives restart');
    expect(lockHasGate(lock)).toBe(false);
  });
  it('treats password, challenge, and delay as gates', () => {
    expect(lockHasGate({ password_hash: 'v1:1:aa:bb' })).toBe(true);
    expect(lockHasGate({ challenge_length: 8 })).toBe(true);
    expect(lockHasGate({ delay_minutes: 1 })).toBe(true);
    expect(lockHasGate({ challenge_length: 0, delay_minutes: 0 })).toBe(false);
  });
});

describe('isScheduleLockedActive with step 10 locks', () => {
  // Monday 2026-10-26 09:30 local: inside the 09:00-10:00 window.
  const now = new Date(2026, 9, 26, 9, 30);
  it('is locked when a step 10 gate is set and the window is active', () => {
    const s: Schedule = {
      ...baseSchedule,
      lock: { challenge_length: 16 },
    };
    expect(isScheduleLockedActive(s, now)).toBe(true);
  });
  it('is not locked when only a restart lock is set', () => {
    const s: Schedule = { ...baseSchedule, lock: { restart_lock: true } };
    expect(isScheduleLockedActive(s, now)).toBe(false);
  });
  it('treats an elapsed change delay as unlocked', () => {
    const s: Schedule = {
      ...baseSchedule,
      lock: { delay_minutes: 5 },
      lock_state: {
        delay_until_ms: now.getTime() - 1000,
        delay_for: 'change',
      },
    };
    expect(isScheduleLockedActive(s, now)).toBe(false);
  });
  it('stays locked while a change delay is pending', () => {
    const s: Schedule = {
      ...baseSchedule,
      lock: { delay_minutes: 5 },
      lock_state: {
        delay_until_ms: now.getTime() + 300_000,
        delay_for: 'change',
      },
    };
    expect(isScheduleLockedActive(s, now)).toBe(true);
  });
});
