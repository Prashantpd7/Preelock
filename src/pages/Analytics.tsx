import { PageHeading, Stat } from '../components/ui';
import { useAppStore } from '../state/store';
import { formatDuration, statistics, week } from '../services/analytics';
export function Analytics() {
  const { sessions, tasks } = useAppStore((s) => s.data);
  const stats = statistics(sessions, tasks);
  const days = week(sessions);
  const max = Math.max(...days.map((d) => d.ms), 60_000);
  return (
    <>
      <PageHeading
        eyebrow="PROGRESS YOU CAN SEE"
        title="Analytics"
        description="Small efforts add up. These numbers come from your saved sessions."
      />
      <div className="stats-grid">
        <Stat
          label="Total focus"
          value={formatDuration(stats.total)}
          detail="Includes time in sessions ended early"
        />
        <Stat
          label="Completed sessions"
          value={String(stats.completed)}
          detail="Sessions that reached their deadline"
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
