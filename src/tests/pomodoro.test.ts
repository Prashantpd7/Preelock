import { expect, test } from 'vitest';
import { defaultFocus, pomodoroLabel } from '../types';

test('pomodoroLabel names each phase', () => {
  expect(
    pomodoroLabel({
      phase: 'focus',
      phase_remaining_ms: 25 * 60_000,
      completed_focus_cycles: 0,
    }),
  ).toBe('Focus');
  expect(
    pomodoroLabel({
      phase: 'short_break',
      phase_remaining_ms: 5 * 60_000,
      completed_focus_cycles: 1,
    }),
  ).toBe('Short break');
  expect(
    pomodoroLabel({
      phase: 'long_break',
      phase_remaining_ms: 15 * 60_000,
      completed_focus_cycles: 4,
    }),
  ).toBe('Long break');
  expect(pomodoroLabel(undefined)).toBe(null);
});

test('defaultFocus ships Pomodoro disabled with classic timings', () => {
  expect(defaultFocus.pomodoro).toEqual({
    enabled: false,
    focus_minutes: 25,
    short_break_minutes: 5,
    long_break_minutes: 15,
    cycles_before_long_break: 4,
  });
});
