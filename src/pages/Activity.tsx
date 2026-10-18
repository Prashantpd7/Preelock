import { PageHeading, Empty } from '../components/ui';
import { useAppStore } from '../state/store';
import { formatDuration } from '../services/analytics';
export function Activity() {
  const sessions = useAppStore((s) => s.data.sessions)
    .filter((s) => s.started_at !== null)
    .slice()
    .reverse();
  return (
    <>
      <PageHeading
        eyebrow="YOUR WORK, OVER TIME"
        title="Activity"
        description="A record of your focus sessions. Device monitoring is not enabled."
      />
      <div className="panel table-panel">
        {sessions.length ? (
          <table>
            <thead>
              <tr>
                <th>Session</th>
                <th>Started</th>
                <th>Focused</th>
                <th>Status</th>
              </tr>
            </thead>
            <tbody>
              {sessions.map((s) => (
                <tr key={s.id}>
                  <td>{s.title}</td>
                  <td>{new Date(s.started_at!).toLocaleString()}</td>
                  <td>{formatDuration(s.focused_ms)}</td>
                  <td>
                    <span className="badge">{s.status}</span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <Empty>
            No activity yet. Start a focus session to build your history.
          </Empty>
        )}
      </div>
    </>
  );
}
