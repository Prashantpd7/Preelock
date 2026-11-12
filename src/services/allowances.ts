import type { Allowance } from '../types';

/** Time allowances (step 8): daily/weekly budgets for one site or
 *  app. These helpers mirror the Rust backend in
 *  src-tauri/src/allowances.rs exactly, so the remaining time shown
 *  here matches what the engine enforces — the backend is always
 *  authoritative, this is display only.
 *
 *  Scope reminder (backend-enforced): budgets deplete only from
 *  live ticks during a Running session in a Focus phase. Break time
 *  never consumes a budget, and nothing accrues or blocks while no
 *  session runs. A period roll, disable, or delete lifts the block
 *  on the next tick/policy publish — exhaustion is computed from
 *  the usage log, never stored. */

/** Start of the current fixed period in local time: local midnight
 *  for a day, local Monday 00:00 for a week. */
export function fixedPeriodStartMs(
  period: 'day' | 'week',
  now: Date = new Date(),
): number {
  const dayStart = new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate(),
  ).getTime();
  if (period === 'day') return dayStart;
  const daysSinceMonday = (now.getDay() + 6) % 7;
  return dayStart - daysSinceMonday * 86_400_000;
}

export function rollingWindowStartMs(
  period: 'day' | 'week',
  nowMs: number,
): number {
  return nowMs - (period === 'day' ? 86_400_000 : 604_800_000);
}

/** Usage in the current period, from the minute buckets. Mirrors
 *  the Rust `used_ms`: fixed sums buckets from the period start,
 *  rolling sums buckets whose minute overlaps the trailing window;
 *  buckets ahead of the current clock are never counted. */
export function allowanceUsedMs(
  allowance: Allowance,
  now: Date = new Date(),
): number {
  const nowMs = now.getTime();
  const buckets = allowance.usage ?? [];
  if (allowance.mode === 'fixed') {
    const start = fixedPeriodStartMs(allowance.period, now);
    return buckets
      .filter((b) => b.minute_ms >= start && b.minute_ms <= nowMs)
      .reduce((sum, b) => sum + b.ms, 0);
  }
  const start = rollingWindowStartMs(allowance.period, nowMs);
  return buckets
    .filter((b) => b.minute_ms + 60_000 > start && b.minute_ms <= nowMs)
    .reduce((sum, b) => sum + b.ms, 0);
}

export function allowanceBudgetMs(allowance: Allowance): number {
  return allowance.budget_minutes * 60_000;
}

export function allowanceRemainingMs(
  allowance: Allowance,
  now: Date = new Date(),
): number {
  return Math.max(
    0,
    allowanceBudgetMs(allowance) - allowanceUsedMs(allowance, now),
  );
}

/** Exhausted means enabled AND used up: disabling or deleting
 *  lifts the block immediately, and a period roll resets usage. */
export function allowanceExhausted(
  allowance: Allowance,
  now: Date = new Date(),
): boolean {
  return (
    allowance.enabled &&
    allowanceUsedMs(allowance, now) >= allowanceBudgetMs(allowance)
  );
}

export function formatAllowanceMinutes(ms: number): string {
  const minutes = Math.ceil(ms / 60_000);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest ? `${hours}h ${rest}m` : `${hours}h`;
}

export function allowancePeriodLabel(allowance: Allowance): string {
  const span = allowance.period === 'day' ? 'daily' : 'weekly';
  return `${span}, ${allowance.mode === 'rolling' ? 'rolling' : 'fixed'}`;
}

/** Client-side form validation, mirroring the backend rules. Empty
 *  string means valid; otherwise the message to show. Site targets
 *  are host-only rules: no search: keywords and no paths (the
 *  backend rejects both, since a keyword has no host to budget and
 *  a path budget could never accrue from host-only engine data). */
export function validateAllowanceForm(input: {
  title: string;
  kind: 'site' | 'app';
  target: string;
  budgetMinutes: number;
  period: 'day' | 'week';
}): string {
  if (!input.title.trim() || input.title.trim().length > 160)
    return 'Use a title between 1 and 160 characters.';
  const target = input.target.trim();
  if (!target) return 'Enter a site or app to allow.';
  if (input.kind === 'site') {
    if (target.toLowerCase().startsWith('search:'))
      return 'A site allowance needs a domain, not a search: keyword.';
    if (target.includes('/'))
      return 'A site allowance needs a plain or wildcard domain, not a domain with a path.';
  }
  if (
    target.length > 1024 ||
    [...target].some((c) => c.charCodeAt(0) < 32 || c.charCodeAt(0) === 127)
  )
    return 'Use an app id up to 1024 characters.';
  const max = input.period === 'day' ? 1440 : 10080;
  if (
    !Number.isInteger(input.budgetMinutes) ||
    input.budgetMinutes < 1 ||
    input.budgetMinutes > max
  )
    return `Allow from 1 to ${max} minutes for a ${input.period === 'day' ? 'daily' : 'weekly'} allowance.`;
  return '';
}
