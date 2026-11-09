import { describe, expect, it } from 'vitest';
import {
  DAY_MS,
  canGoNextWeek,
  dayCategoryTotals,
  formatDayLabel,
  formatSelectedDayLabel,
  localDayStart,
  pickDayForWeek,
  weekDays,
  weekStartMs,
} from '../services/screenTimeWeek';
import type { ScreenTimeBucket } from '../types';
import type { CategoryOverrides } from '../services/appCategories';

function bucket(
  dayMs: number,
  name: string,
  minutes: number,
  kind: 'app' | 'site' = 'app',
  target?: string,
): ScreenTimeBucket {
  return {
    day_ms: dayMs,
    kind,
    target: target ?? `com.example.${name.split(' ')[0].toLowerCase()}`,
    name,
    ms: minutes * 60_000,
  };
}

// 2026-10-29 is a Thursday; its week runs Sun 2026-10-25 → Sat 2026-10-31.
const THURSDAY = localDayStart(new Date(2026, 9, 29));
const SUNDAY = localDayStart(new Date(2026, 9, 25));
const SATURDAY = localDayStart(new Date(2026, 9, 31));
const TUESDAY = localDayStart(new Date(2026, 9, 27));

describe('weekStartMs and weekDays', () => {
  it('returns the Sunday of the week for a weekday', () => {
    expect(weekStartMs(THURSDAY)).toBe(SUNDAY);
  });

  it('returns the day itself for a Sunday and the same Sunday for a Saturday', () => {
    expect(weekStartMs(SUNDAY)).toBe(SUNDAY);
    expect(weekStartMs(SATURDAY)).toBe(SUNDAY);
  });

  it('lists 7 consecutive days, Sunday first', () => {
    const days = weekDays(SUNDAY);
    expect(days).toHaveLength(7);
    expect(days[0]).toBe(SUNDAY);
    expect(days[6]).toBe(SATURDAY);
    days.forEach((day, index) => {
      expect(day).toBe(SUNDAY + index * DAY_MS);
      expect(new Date(day).getDay()).toBe(index);
    });
  });
});

describe('dayCategoryTotals', () => {
  it('splits one day by category and sums to the total', () => {
    const buckets = [
      bucket(THURSDAY, 'Instagram', 60),
      bucket(THURSDAY, 'Terminal', 30),
      bucket(THURSDAY, 'Finder', 10),
      bucket(THURSDAY, 'YouTube', 20, 'site', 'youtube.com'),
      bucket(THURSDAY, 'GitHub', 15, 'site', 'github.com'),
      bucket(THURSDAY, 'Example', 5, 'site', 'example.com'),
      bucket(TUESDAY, 'Instagram', 999),
    ];
    const totals = dayCategoryTotals(buckets, THURSDAY, 'all');
    expect(totals.distracting).toBe(80 * 60_000);
    expect(totals.productive).toBe(45 * 60_000);
    expect(totals.others).toBe(15 * 60_000);
    expect(totals.total).toBe(140 * 60_000);
    expect(totals.distracting + totals.productive + totals.others).toBe(
      totals.total,
    );
  });

  it('respects the kind filter', () => {
    const buckets = [
      bucket(THURSDAY, 'Instagram', 60),
      bucket(THURSDAY, 'YouTube', 20, 'site', 'youtube.com'),
    ];
    expect(dayCategoryTotals(buckets, THURSDAY, 'app').total).toBe(60 * 60_000);
    expect(dayCategoryTotals(buckets, THURSDAY, 'site').total).toBe(
      20 * 60_000,
    );
  });

  it('applies saved corrections, including to a past day', () => {
    const buckets = [
      bucket(TUESDAY, 'Muse', 13),
      bucket(TUESDAY, 'Instagram', 60),
      bucket(THURSDAY, 'Muse', 30),
    ];
    // Muse is Productive by default; correcting it to Distracting
    // moves its minutes on BOTH days, while each day's total is
    // unchanged — a correction re-colors history, it never rewrites it.
    const overrides: CategoryOverrides = {
      'com.example.muse': 'Distracting',
    };
    const past = dayCategoryTotals(buckets, TUESDAY, 'all', overrides);
    expect(past.productive).toBe(0);
    expect(past.distracting).toBe(73 * 60_000);
    expect(past.total).toBe(73 * 60_000);
    const today = dayCategoryTotals(buckets, THURSDAY, 'all', overrides);
    expect(today.distracting).toBe(30 * 60_000);
    expect(today.total).toBe(30 * 60_000);
  });
});

describe('day labels', () => {
  it('formats a day as "Tue, 27 Oct"', () => {
    expect(formatDayLabel(TUESDAY)).toBe('Tue, 27 Oct');
  });

  it('prefixes today with "Today - "', () => {
    expect(formatSelectedDayLabel(THURSDAY, THURSDAY)).toBe(
      'Today - Thu, 29 Oct',
    );
    expect(formatSelectedDayLabel(TUESDAY, THURSDAY)).toBe('Tue, 27 Oct');
  });
});

describe('week navigation rules', () => {
  it('disables next week only on the current week', () => {
    expect(canGoNextWeek(0)).toBe(false);
    expect(canGoNextWeek(-1)).toBe(true);
    expect(canGoNextWeek(-2)).toBe(true);
  });

  it('re-selects today when the new week is the current week', () => {
    const days = weekDays(SUNDAY);
    expect(pickDayForWeek(days, 4, [0, 0, 0, 0, 0, 0, 0], THURSDAY)).toBe(
      THURSDAY,
    );
  });

  it('keeps the same weekday when it has usage', () => {
    const days = weekDays(SUNDAY - 7 * DAY_MS);
    const totals = [0, 0, 0, 0, 45 * 60_000, 0, 0];
    expect(pickDayForWeek(days, 4, totals, THURSDAY)).toBe(days[4]);
  });

  it('falls back to the most recent day with usage', () => {
    const days = weekDays(SUNDAY - 7 * DAY_MS);
    const totals = [10 * 60_000, 0, 0, 0, 0, 20 * 60_000, 0];
    expect(pickDayForWeek(days, 4, totals, THURSDAY)).toBe(days[5]);
  });

  it('keeps the same weekday when the week has no usage at all', () => {
    const days = weekDays(SUNDAY - 7 * DAY_MS);
    expect(pickDayForWeek(days, 4, [0, 0, 0, 0, 0, 0, 0], THURSDAY)).toBe(
      days[4],
    );
  });
});
