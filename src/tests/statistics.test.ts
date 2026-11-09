import { describe, expect, it } from 'vitest';
import { attemptCounts, websiteBlocksTotal } from '../services/analytics';
import { defaultFocus, type Session } from '../types';

function makeSession(over: Partial<Session> = {}): Session {
  return {
    id: 's1',
    title: 'Session',
    duration_minutes: 25,
    remaining_ms: 0,
    focused_ms: 0,
    allow_pause: true,
    status: 'completed',
    started_at: null,
    ended_at: null,
    deadline_ms: null,
    focus: defaultFocus,
    ...over,
  };
}

describe('attemptCounts', () => {
  it('ranks most-blocked first with stable ties', () => {
    const counts = attemptCounts([
      { target: 'b.app', kind: 'app' },
      { target: 'a.app', kind: 'app' },
      { target: 'b.app', kind: 'app' },
      { target: 'a.app', kind: 'app' },
    ]);
    expect(counts).toEqual([
      { target: 'a.app', kind: 'app', count: 2 },
      { target: 'b.app', kind: 'app', count: 2 },
    ]);
  });

  it('is empty with no attempts', () => {
    expect(attemptCounts([])).toEqual([]);
  });
});

describe('websiteBlocksTotal', () => {
  it('sums per-session browser_blocks and ignores missing reports', () => {
    const total = websiteBlocksTotal([
      makeSession({
        report: {
          events: [],
          approved_ms: 0,
          distracting_ms: 0,
          idle_ms: 0,
          interruptions: 0,
          active_app: '',
          enforcement: '',
          camera_status: '',
          browser_blocks: 7,
        },
      }),
      makeSession(),
      makeSession({
        report: {
          events: [],
          approved_ms: 0,
          distracting_ms: 0,
          idle_ms: 0,
          interruptions: 0,
          active_app: '',
          enforcement: '',
          camera_status: '',
          browser_blocks: 3,
        },
      }),
    ]);
    expect(total).toBe(10);
  });
});
