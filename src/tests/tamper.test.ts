import { describe, expect, it } from 'vitest';
import { tamperNote, type Session } from '../types';

function sessionWithEvents(kinds: string[]): Session {
  return {
    id: 's1',
    title: 'Work',
    duration_minutes: 25,
    remaining_ms: 25 * 60_000,
    focused_ms: 0,
    allow_pause: true,
    status: 'running',
    started_at: 0,
    ended_at: null,
    deadline_ms: null,
    report: {
      events: kinds.map((kind, i) => ({
        at: 1000 + i,
        kind,
        detail: `${kind} detail`,
      })),
      approved_ms: 0,
      distracting_ms: 0,
      idle_ms: 0,
      interruptions: 0,
      active_app: '',
      enforcement: '',
      camera_status: '',
      browser_blocks: 0,
    },
  };
}

describe('tamperNote (step 11)', () => {
  it('is null with no report or no tamper events', () => {
    expect(tamperNote(sessionWithEvents([]))).toBeNull();
    expect(
      tamperNote(sessionWithEvents(['browser_disconnected', 'active_app'])),
    ).toBeNull();
    const noReport: Session = { ...sessionWithEvents([]), report: undefined };
    expect(tamperNote(noReport)).toBeNull();
  });
  it('reports the latest tamper event only, concisely', () => {
    expect(tamperNote(sessionWithEvents(['clock_jump']))).toBe(
      'Clock jump noted — jumped time was not credited.',
    );
    expect(tamperNote(sessionWithEvents(['extension_removed']))).toBe(
      'Browser blocked until the extension is connected.',
    );
    // Latest wins, and the full detail is never surfaced.
    const note = tamperNote(
      sessionWithEvents(['clock_jump', 'extension_removed']),
    );
    expect(note).toBe('Browser blocked until the extension is connected.');
    expect(note).not.toContain('detail');
  });
  it('carries the step 11 fail-closed flag on the session type', () => {
    const s: Session = {
      ...sessionWithEvents(['extension_removed']),
      policy_applied: false,
      browser_fail_closed: true,
    };
    expect(s.browser_fail_closed).toBe(true);
    expect(s.policy_applied).toBe(false);
  });
});
