import { useEffect, useState } from 'react';
import { PageHeading, Stat } from '../components/ui';
import { useAppStore } from '../state/store';
import { api, desktop } from '../services/api';
import {
  ANALYTICS_RANGES,
  analyticsSummary,
  attemptCounts,
  formatShortDuration,
  websiteBlocksTotal,
  type AnalyticsRange,
} from '../services/analytics';
import type { BlockAttempt } from '../types';
const RANGE_LABEL: Record<AnalyticsRange, string> = {
  today: 'Today',
  week: 'Last 7 days',
  month: 'This month',
  all: 'All time',
};
export function Analytics() {
  const { sessions } = useAppStore((s) => s.data);
  const [range, setRange] = useState<AnalyticsRange>('week');
  const [attempts, setAttempts] = useState<BlockAttempt[]>([]);
  useEffect(() => {
    if (!desktop) return;
    let live = true;
    api
      .blockAttempts()
      .then((rows) => {
        if (live) setAttempts(rows);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);
  const summary = analyticsSummary(sessions, range);
  const counts = attemptCounts(attempts);
  const max = Math.max(...summary.days.map((d) => d.ms), 60_000);
  const compactChart = summary.days.length > 14;
  return (
    <>
      <PageHeading
        eyebrow=""
        title="Analytics"
        description="Your focus progress."
      />
      <div
        className="segmented analytics-filter"
        role="group"
        aria-label="Date range"
      >
        {ANALYTICS_RANGES.map((r) => (
          <button
            key={r.value}
            type="button"
            aria-pressed={range === r.value}
            onClick={() => setRange(r.value)}
          >
            {r.label}
          </button>
        ))}
      </div>
      <div className="stats-grid analytics-grid">
        <Stat
          label="Focus time"
          value={formatShortDuration(summary.focusMs)}
          detail={RANGE_LABEL[range]}
        />
        <Stat
          label="Sessions completed"
          value={`${summary.completedCount}/${summary.finishedCount}`}
          detail="Reached their target"
        />
        <Stat
          label="Days focused"
          value={`${summary.daysFocused}/${summary.dayCount}`}
          detail="Days with focus time"
        />
        <Stat
          label="Average session"
          value={
            summary.averageMs == null
              ? '—'
              : formatShortDuration(summary.averageMs)
          }
          detail="Per finished session"
        />
        <Stat
          label="Best session"
          value={
            summary.best ? formatShortDuration(summary.best.focused_ms) : '—'
          }
          detail={summary.best ? summary.best.title : 'No finished sessions'}
          detailTitle={summary.best?.title}
          truncateDetail
        />
        <Stat
          label="Longest streak"
          value={`${summary.longestStreakDays} day${summary.longestStreakDays === 1 ? '' : 's'}`}
          detail="Consecutive focus days"
        />
      </div>
      <details className="panel breakdown">
        <summary>Time breakdown</summary>
        <dl>
          <div>
            <dt>Total session time</dt>
            <dd>{formatShortDuration(summary.breakdown.elapsedMs)}</dd>
          </div>
          <div>
            <dt>Distracted time</dt>
            <dd>{formatShortDuration(summary.breakdown.distractedMs)}</dd>
          </div>
          <div>
            <dt>Away time</dt>
            <dd>{formatShortDuration(summary.breakdown.awayMs)}</dd>
          </div>
          <div>
            <dt>Unverified time</dt>
            <dd>{formatShortDuration(summary.breakdown.unknownMs)}</dd>
          </div>
        </dl>
        <p className="muted small">
          Pauses included; unverified time excluded.
        </p>
      </details>
      <section className="panel chart-panel">
        <h2>{RANGE_LABEL[range]}</h2>
        <p className="muted small">Focus time split by day.</p>
        <div
          className={`chart${compactChart ? ' scroll-x' : ''}`}
          role="img"
          aria-label={summary.days
            .map(
              (d) =>
                `${d.date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}: ${formatShortDuration(d.ms)}`,
            )
            .join(', ')}
        >
          {summary.days.map((d) => (
            <div
              className="chart-column"
              key={d.key}
              title={d.date.toLocaleDateString(undefined, {
                weekday: 'short',
                month: 'short',
                day: 'numeric',
              })}
            >
              <span className="small">{formatShortDuration(d.ms)}</span>
              <div className="bar-track">
                <div
                  className="bar"
                  style={{ height: `${(d.ms / max) * 100}%` }}
                />
              </div>
              <span className="muted small">
                {compactChart ? d.date.getDate() : d.label}
              </span>
            </div>
          ))}
        </div>
        {!summary.hasEligible && (
          <p className="muted small">No sessions yet.</p>
        )}
      </section>
      <section className="panel attempts-panel">
        <h2>Block attempts</h2>
        <p className="muted small section-desc">
          Apps blocked: {attempts.length} attempt
          {attempts.length === 1 ? '' : 's'} · Website blocks:{' '}
          {websiteBlocksTotal(sessions)} in total. Website blocks counted in
          total only.
        </p>
        {attempts.length === 0 ? (
          <p className="muted small">No attempts yet.</p>
        ) : (
          <>
            <dl>
              {counts.slice(0, 10).map((entry) => (
                <div key={`${entry.kind}:${entry.target}`}>
                  <dt>{entry.target}</dt>
                  <dd>
                    {entry.count} attempt{entry.count === 1 ? '' : 's'}
                  </dd>
                </div>
              ))}
            </dl>
            <h3>Recent attempts</h3>
            <ul>
              {attempts.slice(0, 20).map((attempt, index) => (
                <li key={`${attempt.at_ms}-${attempt.target}-${index}`}>
                  {new Date(attempt.at_ms).toLocaleString()} · {attempt.target}{' '}
                  · {attempt.reason}
                </li>
              ))}
            </ul>
          </>
        )}
      </section>
    </>
  );
}
