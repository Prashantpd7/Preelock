import { expect, test } from 'vitest';
import { defaultFocus } from '../types';
import type { RewardState } from '../types';

test('defaultFocus ships reward breaks disabled with 25-for-5 earning', () => {
  expect(defaultFocus.reward).toEqual({
    enabled: false,
    earn_focus_minutes: 25,
    earn_break_minutes: 5,
    max_credit_minutes: 30,
    max_break_minutes: 30,
    delay_minutes: 0,
  });
});

test('reward state encoding: one field at a time means one break state', () => {
  const idle: RewardState = {
    credit_ms: 5 * 60_000,
    earn_progress_ms: 0,
    pending_delay_ms: 0,
    break_remaining_ms: 0,
  };
  expect(idle.pending_delay_ms).toBe(0);
  expect(idle.break_remaining_ms).toBe(0);
  const pending: RewardState = { ...idle, pending_delay_ms: 5 * 60_000 };
  expect(pending.pending_delay_ms).toBeGreaterThan(0);
  expect(pending.break_remaining_ms).toBe(0);
  const onBreak: RewardState = { ...idle, break_remaining_ms: 5 * 60_000 };
  expect(onBreak.break_remaining_ms).toBeGreaterThan(0);
  expect(onBreak.pending_delay_ms).toBe(0);
});
