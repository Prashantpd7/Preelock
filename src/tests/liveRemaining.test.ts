import { describe, expect, it } from 'vitest';
import { liveRemainingMs } from '../services/activeSession';
import type { Session } from '../types';

function session(overrides: Partial<Session>): Session {
  return {
    id: 's',
    title: 'Focus',
    duration_minutes: 1,
    remaining_ms: 60_000,
    focused_ms: 0,
    allow_pause: true,
    status: 'running',
    started_at: 1_000_000,
    ended_at: null,
    deadline_ms: 1_060_000,
    ...overrides,
  } as Session;
}

describe('liveRemainingMs', () => {
  it('ticks from a stored focused-time safety deadline', () => {
    const s = session({});
    expect(liveRemainingMs(s, 1_000_000)).toBe(60_000);
    expect(liveRemainingMs(s, 1_030_000)).toBe(30_000);
    expect(liveRemainingMs(s, 1_059_000)).toBe(1_000);
  });

  it('keeps moving even when no snapshot has arrived (engine stalled)', () => {
    // Same frozen snapshot object, two different local times: the
    // countdown must still change. This is the 00:58-forever bug.
    const frozenSnapshot = session({
      remaining_ms: 58_000,
      deadline_ms: 1_058_000,
    });
    expect(liveRemainingMs(frozenSnapshot, 1_002_000)).toBe(56_000);
    expect(liveRemainingMs(frozenSnapshot, 1_012_000)).toBe(46_000);
  });

  it('never goes negative after the deadline', () => {
    expect(liveRemainingMs(session({ deadline_ms: 1_000 }), 2_000_000)).toBe(0);
  });

  it('uses the snapshot value while paused and during grace', () => {
    expect(
      liveRemainingMs(
        session({ status: 'paused', remaining_ms: 42_000 }),
        5_000_000,
      ),
    ).toBe(42_000);
    expect(
      liveRemainingMs(
        session({ grace_until_ms: 1_010_000, remaining_ms: 60_000 }),
        1_005_000,
      ),
    ).toBe(60_000);
  });
});
