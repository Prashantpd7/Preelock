import type { ScreenTimeBucket } from '../types';
import { appCategory, type CategoryOverrides } from './appCategories';

// Pure week/day helpers for the Screen Time chart. The chart always shows
// one full week, Sunday to Saturday, and the list below it always shows
// the selected day's usage (never a range total).

export const DAY_MS = 86_400_000;

export const WEEKDAY_LETTERS = ['S', 'M', 'T', 'W', 'T', 'F', 'S'];
export const WEEKDAY_SHORT = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
export const MONTH_SHORT = [
  'Jan',
  'Feb',
  'Mar',
  'Apr',
  'May',
  'Jun',
  'Jul',
  'Aug',
  'Sep',
  'Oct',
  'Nov',
  'Dec',
];

export type UsageKind = 'all' | 'app' | 'site';

export interface DayCategoryTotals {
  distracting: number;
  productive: number;
  others: number;
  total: number;
}

// Local-midnight timestamp for the given date (matches the backend's
// day_ms, which is also a local day start).
export function localDayStart(date: Date): number {
  return new Date(
    date.getFullYear(),
    date.getMonth(),
    date.getDate(),
  ).getTime();
}

// Local-calendar day shift so DST transitions (23h or 25h days) always
// land on local midnight `00:00:00.000` instead of drifting by an hour.
export function shiftDays(dayMs: number, days: number): number {
  const date = new Date(dayMs);
  return new Date(
    date.getFullYear(),
    date.getMonth(),
    date.getDate() + days,
  ).getTime();
}

// Sunday (local) starting the week that contains dayMs, where dayMs is a
// local-midnight timestamp.
export function weekStartMs(dayMs: number): number {
  return shiftDays(dayMs, -new Date(dayMs).getDay());
}

// The 7 local-midnight timestamps of the week starting at weekStart,
// Sunday first.
export function weekDays(weekStart: number): number[] {
  return Array.from({ length: 7 }, (_, index) => shiftDays(weekStart, index));
}

// A day's usage split by category, reusing the list's own mapping so the
// chart and the list can never disagree. The three splits always sum to
// total. Overrides (the user's corrections) apply here too, so a
// re-categorized app re-colors past days as well as today.
export function dayCategoryTotals(
  buckets: ScreenTimeBucket[],
  dayMs: number,
  kind: UsageKind,
  overrides?: CategoryOverrides,
): DayCategoryTotals {
  const totals: DayCategoryTotals = {
    distracting: 0,
    productive: 0,
    others: 0,
    total: 0,
  };
  for (const bucket of buckets) {
    if (bucket.day_ms !== dayMs) continue;
    if (kind !== 'all' && bucket.kind !== kind) continue;
    const category = appCategory(
      bucket.kind,
      bucket.target,
      bucket.name || bucket.target,
      overrides,
    );
    if (category === 'Distracting') totals.distracting += bucket.ms;
    else if (category === 'Productive') totals.productive += bucket.ms;
    else totals.others += bucket.ms;
    totals.total += bucket.ms;
  }
  return totals;
}

// "Tue, 27 Oct"
export function formatDayLabel(dayMs: number): string {
  const day = new Date(dayMs);
  return `${WEEKDAY_SHORT[day.getDay()]}, ${day.getDate()} ${MONTH_SHORT[day.getMonth()]}`;
}

// "Today - Thu, 29 Oct" when the day is today, otherwise "Tue, 27 Oct".
export function formatSelectedDayLabel(dayMs: number, todayMs: number): string {
  if (dayMs === todayMs) return `Today - ${formatDayLabel(dayMs)}`;
  return formatDayLabel(dayMs);
}

// The next-week arrow is disabled when the shown week is the current week.
export function canGoNextWeek(weekOffset: number): boolean {
  return weekOffset < 0;
}

// Which day to select when moving to a new week: today when the new week
// is the current week; otherwise the same weekday if it has usage, else
// the week's most recent day with usage, else the same weekday (an empty
// day — shown honestly as 0m rather than jumping somewhere surprising).
export function pickDayForWeek(
  days: number[],
  sameWeekdayIndex: number,
  totals: number[],
  todayMs: number,
): number {
  const todayIndex = days.indexOf(todayMs);
  if (todayIndex >= 0) return todayMs;
  if (totals[sameWeekdayIndex] > 0) return days[sameWeekdayIndex];
  let lastWithData = -1;
  totals.forEach((total, index) => {
    if (total > 0) lastWithData = index;
  });
  return lastWithData >= 0 ? days[lastWithData] : days[sameWeekdayIndex];
}
