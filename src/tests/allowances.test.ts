import { describe, expect, it } from 'vitest';
import {
  allowanceExhausted,
  allowanceRemainingMs,
  allowanceUsedMs,
  validateAllowanceForm,
} from '../services/allowances';
import type { Allowance } from '../types';

function allowance(partial: Partial<Allowance> = {}): Allowance {
  return {
    id: 'a1',
    title: 'YouTube',
    kind: 'site',
    target: 'youtube.com',
    budget_minutes: 30,
    period: 'day',
    mode: 'fixed',
    enabled: true,
    usage: [],
    created_at: 0,
    ...partial,
  };
}

describe('allowance usage math', () => {
  it('depletes and exhausts at exactly the budget', () => {
    const now = new Date(2026, 9, 27, 10, 0, 0);
    const a = allowance({
      usage: [
        {
          minute_ms: new Date(2026, 9, 27, 9, 0, 0).getTime(),
          ms: 25 * 60_000,
        },
        {
          minute_ms: new Date(2026, 9, 27, 9, 30, 0).getTime(),
          ms: 5 * 60_000,
        },
      ],
    });
    expect(allowanceUsedMs(a, now)).toBe(30 * 60_000);
    expect(allowanceRemainingMs(a, now)).toBe(0);
    expect(allowanceExhausted(a, now)).toBe(true);
  });

  it('fixed day resets at local midnight', () => {
    const now = new Date(2026, 9, 27, 10, 0, 0);
    const a = allowance({
      usage: [
        {
          minute_ms: new Date(2026, 9, 26, 23, 0, 0).getTime(),
          ms: 30 * 60_000,
        },
      ],
    });
    expect(allowanceUsedMs(a, now)).toBe(0);
    expect(allowanceExhausted(a, now)).toBe(false);
  });

  it('fixed week starts on Monday', () => {
    // 2026-10-27 is a Tuesday; the previous Sunday belongs to last week.
    const now = new Date(2026, 9, 27, 10, 0, 0);
    const sunday = allowance({
      usage: [
        {
          minute_ms: new Date(2026, 9, 25, 12, 0, 0).getTime(),
          ms: 30 * 60_000,
        },
      ],
    });
    expect(allowanceUsedMs(sunday, now)).toBe(0);
    const monday = allowance({
      period: 'week',
      usage: [
        {
          minute_ms: new Date(2026, 9, 26, 12, 0, 0).getTime(),
          ms: 30 * 60_000,
        },
      ],
    });
    expect(allowanceUsedMs(monday, now)).toBe(30 * 60_000);
  });

  it('rolling windows expire old usage', () => {
    const now = new Date(2026, 9, 27, 10, 0, 0);
    const old = allowance({
      mode: 'rolling',
      usage: [
        {
          minute_ms: new Date(2026, 9, 26, 9, 0, 0).getTime(),
          ms: 30 * 60_000,
        },
      ],
    });
    expect(allowanceUsedMs(old, now)).toBe(0);
    const recent = allowance({
      mode: 'rolling',
      usage: [
        {
          minute_ms: new Date(2026, 9, 26, 11, 0, 0).getTime(),
          ms: 30 * 60_000,
        },
      ],
    });
    expect(allowanceUsedMs(recent, now)).toBe(30 * 60_000);
    expect(allowanceExhausted(recent, now)).toBe(true);
  });

  it('disabling lifts exhaustion immediately', () => {
    const now = new Date(2026, 9, 27, 10, 0, 0);
    const a = allowance({
      enabled: false,
      usage: [
        {
          minute_ms: new Date(2026, 9, 27, 9, 0, 0).getTime(),
          ms: 30 * 60_000,
        },
      ],
    });
    expect(allowanceUsedMs(a, now)).toBe(30 * 60_000);
    expect(allowanceExhausted(a, now)).toBe(false);
  });
});

describe('allowance form validation', () => {
  const base = {
    title: 'YouTube',
    kind: 'site' as const,
    target: 'youtube.com',
    budgetMinutes: 30,
    period: 'day' as const,
  };
  it('rejects keyword and path site targets', () => {
    expect(
      validateAllowanceForm({ ...base, target: 'search:chess' }),
    ).toContain('search:');
    expect(
      validateAllowanceForm({ ...base, target: 'youtube.com/shorts' }),
    ).toContain('path');
  });
  it('rejects budgets longer than the period', () => {
    expect(validateAllowanceForm({ ...base, budgetMinutes: 1441 })).toContain(
      '1440',
    );
    expect(
      validateAllowanceForm({
        ...base,
        period: 'week',
        budgetMinutes: 10080,
      }),
    ).toBe('');
  });
  it('uses plain allow wording in validation messages', () => {
    expect(validateAllowanceForm({ ...base, target: '' })).toBe(
      'Enter a site or app to allow.',
    );
    expect(validateAllowanceForm({ ...base, budgetMinutes: 1441 })).toContain(
      'Allow from 1 to 1440 minutes',
    );
  });
  it('accepts wildcard domains and app targets', () => {
    expect(validateAllowanceForm({ ...base, target: '*.example.com' })).toBe(
      '',
    );
    expect(
      validateAllowanceForm({
        ...base,
        kind: 'app',
        target: 'com.example.game',
      }),
    ).toBe('');
  });
});
