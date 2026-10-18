import { PageHeading, Empty } from '../components/ui';
import { SessionCard } from '../components/SessionCard';
import { useAppStore } from '../state/store';
export function Activity() {
  const sessions = useAppStore((s) => s.data.sessions)
    .filter((s) => s.started_at !== null || s.status === 'interrupted')
    .slice()
    .reverse();
  return (
    <>
      <PageHeading
        eyebrow="YOUR WORK, OVER TIME"
        title="Activity"
        description="Local session summaries, consented activity and recovery events."
      />
      <div className="session-list">
        {sessions.length ? (
          sessions.map((s) => <SessionCard key={s.id} session={s} />)
        ) : (
          <div className="panel">
            <Empty>
              No activity yet. Start a focus session to build your history.
            </Empty>
          </div>
        )}
      </div>
    </>
  );
}
