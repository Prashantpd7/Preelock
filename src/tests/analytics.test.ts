import { describe, expect, it } from 'vitest';
import { countdown, statistics, week } from '../services/analytics';
import type { Session } from '../types';
const now = new Date(2026, 9, 18, 12).getTime();
const session: Session = {
  id: 's',
  title: 'Read',
  duration_minutes: 25,
  remaining_ms: 0,
  focused_ms: 1_500_000,
  allow_pause: true,
  status: 'completed',
  started_at: now,
  ended_at: now + 1_500_000,
  deadline_ms: null,
};
describe('actual session analytics', () => {
  it('has honest empty statistics', () => {
    expect(statistics([], [], now)).toEqual({
      today: 0,
      total: 0,
      completed: 0,
      tasksDone: 0,
    });
  });
  it('counts partial focus and uses session start date', () => {
    const previous = {
      ...session,
      id: 'older',
      started_at: now - 86_400_000,
      status: 'ended' as const,
      focused_ms: 30_000,
    };
    expect(statistics([session, previous], [], now)).toEqual({
      today: 1_500_000,
      total: 1_530_000,
      completed: 1,
      tasksDone: 0,
    });
    expect(week([session], new Date(now)).at(-1)?.ms).toBe(1_500_000);
  });
  it('rounds the countdown up and clamps expired sessions', () => {
    expect(countdown(59_001)).toBe('01:00');
    expect(countdown(-1)).toBe('00:00');
  });
});
