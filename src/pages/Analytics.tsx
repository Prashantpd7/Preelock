import { PageHeading, Stat } from '../components/ui';
import { useAppStore } from '../state/store';
import {
  formatDuration,
  productiveStatistics,
  statistics,
  week,
} from '../services/analytics';
export function Analytics() {
  const { sessions, tasks } = useAppStore((s) => s.data);
  const stats = statistics(sessions, tasks);
  const productive = productiveStatistics(sessions);
  const days = week(sessions);
  const max = Math.max(...days.map((d) => d.ms), 60_000);
  return (
    <>
      <PageHeading
        eyebrow="PROGRESS YOU CAN SEE"
        title="Analytics"
        description="Your focus progress."
      />
      <div className="stats-grid">
        <Stat
          label="Focus time"
          value={formatDuration(productive.accepted)}
          detail="Based on your focus checks"
        />
        <Stat
          label="Total session time"
          value={formatDuration(productive.elapsed)}
          detail="Includes time the timer was paused"
        />
        <Stat
          label="Distracted time"
          value={formatDuration(productive.distracted)}
          detail={`${productive.percentage}% focus · ${formatDuration(productive.unknown)} unverified`}
        />
      </div>
      <div className="stats-grid">
        <Stat
          label="All recorded focus"
          value={formatDuration(stats.total)}
          detail="Includes legacy elapsed-time sessions"
        />
        <Stat
          label="Completed sessions"
          value={String(stats.completed)}
          detail="Sessions that reached their target"
        />
        <Stat
          label="Tasks completed"
          value={String(stats.tasksDone)}
          detail="Manually marked complete by you"
        />
      </div>
      <section className="panel chart-panel">
        <h2>Last seven days</h2>
        <p className="muted small">
          Focus time is assigned to the local date each session started.
        </p>
        <div
          className="chart"
          role="img"
          aria-label={days
            .map((d) => `${d.label}: ${formatDuration(d.ms)}`)
            .join(', ')}
        >
          {days.map((d, i) => (
            <div className="chart-column" key={i}>
              <span className="small">{formatDuration(d.ms)}</span>
              <div className="bar-track">
                <div
                  className="bar"
                  style={{ height: `${(d.ms / max) * 100}%` }}
                />
              </div>
              <span className="muted small">{d.label}</span>
            </div>
          ))}
        </div>
        {!stats.total && (
          <p className="muted small">
            Your first session will bring this chart to life.
          </p>
        )}
      </section>
    </>
  );
}
