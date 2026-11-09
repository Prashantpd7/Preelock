import { useId, useRef, useState } from 'react';
import { PageHeading } from '../components/ui';
import { AppLockPanel } from '../components/AppLockPanel';
import { useAppStore } from '../state/store';
import { desktop } from '../services/api';
import {
  appCategory,
  categoryKey,
  type AppCategory,
  type CategoryOverrides,
} from '../services/appCategories';
import { RowIcon } from '../components/AppIcon';
import type { ScreenTimeBucket } from '../types';
import {
  DAY_MS,
  WEEKDAY_LETTERS,
  canGoNextWeek,
  dayCategoryTotals,
  formatDayLabel,
  formatSelectedDayLabel,
  localDayStart,
  pickDayForWeek,
  weekDays,
  weekStartMs,
  type UsageKind,
} from '../services/screenTimeWeek';

const EMPTY_BUCKETS: ScreenTimeBucket[] = [];

// The three categories, in the order the correction menu offers them.
const CATEGORIES: AppCategory[] = ['Distracting', 'Productive', 'Others'];

function duration(ms: number) {
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest ? `${hours}h ${rest}m` : `${hours}h`;
}

export function ScreenTime() {
  const { data, execute } = useAppStore();
  const buckets = data.screen_time_usage ?? EMPTY_BUCKETS;
  // The user's category corrections, from preferences (persisted via
  // the backend settings store). Every categorization on this page —
  // list, chart, and totals — reads this one map, so a correction
  // re-colors past days as well as today.
  const overrides: CategoryOverrides =
    data.preferences.category_overrides ?? {};
  const [kind, setKind] = useState<UsageKind>('all');
  const [expanded, setExpanded] = useState(false);
  const [weekOffset, setWeekOffset] = useState(0);
  const [selectedDay, setSelectedDay] = useState(() =>
    localDayStart(new Date()),
  );
  // At most one category menu open at a time, keyed by row
  // (`${kind}:${target}`). Mirrors the shared Dropdown's listbox/option
  // pattern: the trigger keeps focus while open and tracks the
  // highlighted option with aria-activedescendant.
  const [menuKey, setMenuKey] = useState<string | null>(null);
  const [menuActive, setMenuActive] = useState<number | null>(null);
  const triggerRefs = useRef(new Map<string, HTMLButtonElement>());
  const rawId = useId().replace(/:/g, '');
  const menuListboxId = `screen-time-category-${rawId}-listbox`;
  const menuOptionId = (index: number) =>
    `screen-time-category-${rawId}-option-${index}`;

  const today = localDayStart(new Date());
  const weekStart = weekStartMs(today + weekOffset * 7 * DAY_MS);
  const days = weekDays(weekStart);
  const dayTotals = days.map((day) => ({
    day,
    ...dayCategoryTotals(buckets, day, kind, overrides),
  }));
  const selectedTotals = dayTotals.find(
    (entry) => entry.day === selectedDay,
  ) ?? {
    day: selectedDay,
    distracting: 0,
    productive: 0,
    others: 0,
    total: 0,
  };
  const maxDay = Math.max(...dayTotals.map((entry) => entry.total), 60_000);

  // The list follows the selected day: only that day's usage, never a
  // range total, so the list and the chart above it always agree.
  const dayBuckets = buckets.filter(
    (bucket) =>
      bucket.day_ms === selectedDay && (kind === 'all' || bucket.kind === kind),
  );
  const grouped = new Map<string, ScreenTimeBucket>();
  for (const bucket of dayBuckets) {
    const key = `${bucket.kind}:${bucket.target}`;
    const previous = grouped.get(key);
    if (previous) previous.ms += bucket.ms;
    else grouped.set(key, { ...bucket });
  }
  const byTarget = [...grouped.values()].sort((a, b) => b.ms - a.ms);

  function selectDay(day: number) {
    setSelectedDay(day);
    setExpanded(false);
    setMenuKey(null);
    setMenuActive(null);
  }

  function goWeek(delta: number) {
    const nextOffset = weekOffset + delta;
    if (nextOffset > 0) return;
    const nextWeekStart = weekStartMs(today + nextOffset * 7 * DAY_MS);
    const nextDays = weekDays(nextWeekStart);
    const nextTotals = nextDays.map(
      (day) => dayCategoryTotals(buckets, day, kind, overrides).total,
    );
    const prevWeekday = new Date(selectedDay).getDay();
    setWeekOffset(nextOffset);
    setSelectedDay(pickDayForWeek(nextDays, prevWeekday, nextTotals, today));
    setExpanded(false);
    setMenuKey(null);
    setMenuActive(null);
  }

  function closeMenu(focusTrigger: boolean, key: string | null) {
    setMenuKey(null);
    setMenuActive(null);
    if (focusTrigger && key) triggerRefs.current.get(key)?.focus();
  }

  function openMenu(key: string, current: AppCategory) {
    setMenuKey(key);
    setMenuActive(CATEGORIES.indexOf(current));
  }

  // Save one correction: it lands in preferences via the backend
  // settings store, so it survives restarts, and the store's refreshed
  // snapshot re-renders the row, the chart, and the totals at once.
  function pickCategory(entry: ScreenTimeBucket, category: AppCategory) {
    const key = categoryKey(entry.kind, entry.target);
    closeMenu(true, `${entry.kind}:${entry.target}`);
    void execute({
      type: 'update_preferences',
      preferences: {
        ...data.preferences,
        category_overrides: { ...overrides, [key]: category },
      },
    });
  }

  function onCategoryKeyDown(
    e: React.KeyboardEvent,
    rowKey: string,
    entry: ScreenTimeBucket,
    category: AppCategory,
  ) {
    if (menuKey !== rowKey) {
      if (
        e.key === 'ArrowDown' ||
        e.key === 'ArrowUp' ||
        e.key === 'Enter' ||
        e.key === ' '
      ) {
        e.preventDefault();
        openMenu(rowKey, category);
      }
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setMenuActive((current) =>
        Math.min((current ?? 0) + 1, CATEGORIES.length - 1),
      );
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setMenuActive((current) => Math.max((current ?? 0) - 1, 0));
    } else if (e.key === 'Home') {
      e.preventDefault();
      setMenuActive(0);
    } else if (e.key === 'End') {
      e.preventDefault();
      setMenuActive(CATEGORIES.length - 1);
    } else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      const index = menuActive ?? CATEGORIES.indexOf(category);
      pickCategory(entry, CATEGORIES[index]);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      closeMenu(true, rowKey);
    }
  }

  return (
    <>
      <PageHeading title="Screen Time" />
      {!desktop && (
        <div className="notice">
          Screen Time collection and enforcement are available in the desktop
          app.
        </div>
      )}
      <div
        className="segmented analytics-filter"
        role="group"
        aria-label="Usage type"
      >
        {(['all', 'app', 'site'] as const).map((option) => (
          <button
            type="button"
            key={option}
            aria-pressed={kind === option}
            onClick={() => {
              setKind(option);
              setExpanded(false);
              setMenuKey(null);
              setMenuActive(null);
            }}
          >
            {option === 'all'
              ? 'Everything'
              : option === 'app'
                ? 'Apps'
                : 'Websites'}
          </button>
        ))}
      </div>
      <section className="panel chart-panel screen-time-chart">
        <div className="screen-time-week-nav">
          <button
            type="button"
            className="screen-time-week-arrow"
            aria-label="Previous week"
            onClick={() => goWeek(-1)}
          >
            ‹
          </button>
          <div className="screen-time-week-total">
            <strong>{duration(selectedTotals.total)}</strong>
            <span className="muted small">
              {formatSelectedDayLabel(selectedDay, today)}
            </span>
          </div>
          <button
            type="button"
            className="screen-time-week-arrow"
            aria-label="Next week"
            disabled={!canGoNextWeek(weekOffset)}
            onClick={() => goWeek(1)}
          >
            ›
          </button>
        </div>
        <div
          className="screen-time-week-chart"
          role="group"
          aria-label="Daily usage for this week"
        >
          {dayTotals.map(({ day, distracting, productive, others, total }) => {
            const isSelected = day === selectedDay;
            const date = new Date(day);
            const height = (ms: number) => `${(ms / maxDay) * 100}%`;
            return (
              <button
                type="button"
                key={day}
                className={`screen-time-day${isSelected ? ' selected' : ''}`}
                aria-pressed={isSelected}
                aria-label={formatDayLabel(day)}
                title={`${formatDayLabel(day)}: ${duration(total)}`}
                onClick={() => selectDay(day)}
              >
                <span className="screen-time-bar-track">
                  {isSelected ? (
                    <>
                      <span
                        className="screen-time-seg screen-time-seg-distracting"
                        style={{ height: height(distracting) }}
                      />
                      <span
                        className="screen-time-seg screen-time-seg-productive"
                        style={{ height: height(productive) }}
                      />
                      <span
                        className="screen-time-seg screen-time-seg-others"
                        style={{ height: height(others) }}
                      />
                    </>
                  ) : (
                    <span
                      className="screen-time-seg screen-time-seg-muted"
                      style={{ height: height(total) }}
                    />
                  )}
                </span>
                <span className="screen-time-day-label">
                  {WEEKDAY_LETTERS[date.getDay()]}
                </span>
                <span className="screen-time-day-date">{date.getDate()}</span>
              </button>
            );
          })}
        </div>
        <div className="screen-time-category-totals">
          <span>
            <span
              aria-hidden="true"
              className="screen-time-dot screen-time-dot-distracting"
            />
            <strong>{duration(selectedTotals.distracting)}</strong>
            Distracting
          </span>
          <span>
            <span
              aria-hidden="true"
              className="screen-time-dot screen-time-dot-productive"
            />
            <strong>{duration(selectedTotals.productive)}</strong>
            Productive
          </span>
          <span>
            <span
              aria-hidden="true"
              className="screen-time-dot screen-time-dot-others"
            />
            <strong>{duration(selectedTotals.others)}</strong>
            Others
          </span>
        </div>
      </section>
      <section className="panel screen-time-list">
        <h2>Apps and websites</h2>
        {byTarget.length ? (
          <>
            <ol>
              {(expanded ? byTarget : byTarget.slice(0, 4)).map((entry) => {
                const category = appCategory(
                  entry.kind,
                  entry.target,
                  entry.name || entry.target,
                  overrides,
                );
                const rowKey = `${entry.kind}:${entry.target}`;
                const menuOpen = menuKey === rowKey;
                return (
                  <li key={rowKey}>
                    <div className="screen-time-row">
                      <RowIcon
                        kind={entry.kind}
                        target={entry.target}
                        name={entry.name || entry.target}
                      />
                      <span className="screen-time-name">
                        <strong>{entry.name || entry.target}</strong>
                        <span className="screen-time-category-wrap">
                          <button
                            type="button"
                            className="screen-time-category"
                            ref={(el) => {
                              if (el) triggerRefs.current.set(rowKey, el);
                              else triggerRefs.current.delete(rowKey);
                            }}
                            aria-haspopup="listbox"
                            aria-expanded={menuOpen}
                            aria-controls={menuOpen ? menuListboxId : undefined}
                            aria-activedescendant={
                              menuOpen && menuActive !== null
                                ? menuOptionId(menuActive)
                                : undefined
                            }
                            title="Change category"
                            onClick={() => {
                              if (menuOpen) closeMenu(false, rowKey);
                              else openMenu(rowKey, category);
                            }}
                            onKeyDown={(e) =>
                              onCategoryKeyDown(e, rowKey, entry, category)
                            }
                            onBlur={(e) => {
                              if (
                                !e.currentTarget.parentElement?.contains(
                                  e.relatedTarget as Node | null,
                                )
                              ) {
                                closeMenu(false, rowKey);
                              }
                            }}
                          >
                            <span
                              aria-hidden="true"
                              className={`screen-time-dot screen-time-dot-${category.toLowerCase()}`}
                            />
                            {category}
                          </button>
                          {menuOpen && (
                            <>
                              <div
                                className="screen-time-category-backdrop"
                                onClick={() => closeMenu(true, rowKey)}
                              />
                              <div
                                className="screen-time-category-menu"
                                role="listbox"
                                id={menuListboxId}
                                aria-label={`Category for ${entry.name || entry.target}`}
                              >
                                {CATEGORIES.map((option, index) => (
                                  <button
                                    type="button"
                                    key={option}
                                    id={menuOptionId(index)}
                                    className={[
                                      'screen-time-category-option',
                                      option === category ? 'selected' : '',
                                      index === menuActive ? 'active' : '',
                                    ]
                                      .filter(Boolean)
                                      .join(' ')}
                                    role="option"
                                    aria-selected={option === category}
                                    tabIndex={-1}
                                    onClick={() => pickCategory(entry, option)}
                                    onMouseEnter={() => setMenuActive(index)}
                                  >
                                    <span
                                      aria-hidden="true"
                                      className={`screen-time-dot screen-time-dot-${option.toLowerCase()}`}
                                    />
                                    {option}
                                    {option === category && (
                                      <span
                                        aria-hidden="true"
                                        className="screen-time-category-check"
                                      >
                                        ✓
                                      </span>
                                    )}
                                  </button>
                                ))}
                              </div>
                            </>
                          )}
                        </span>
                      </span>
                      <strong>{duration(entry.ms)}</strong>
                    </div>
                    <div className="screen-time-meter">
                      <span
                        style={{
                          width: `${Math.max(1, (entry.ms / (byTarget[0]?.ms || 1)) * 100)}%`,
                        }}
                      />
                    </div>
                  </li>
                );
              })}
            </ol>
            {byTarget.length > 4 && (
              <button
                type="button"
                className="screen-time-show-all"
                aria-expanded={expanded}
                onClick={() => setExpanded((current) => !current)}
              >
                {expanded ? 'Show less' : 'Show all'}
              </button>
            )}
          </>
        ) : (
          <p className="muted small">No matching usage yet.</p>
        )}
      </section>
      <AppLockPanel />
    </>
  );
}
