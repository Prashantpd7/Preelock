import { useState } from 'react';
import { PageHeading, Empty } from '../components/ui';
import { SessionCard } from '../components/SessionCard';
import { Dropdown } from '../components/Dropdown';
import { PlanningSections } from '../components/PlanningSections';
import { useAppStore } from '../state/store';
import { useFocusSetup } from '../hooks/useFocusSetup';
import { ANALYTICS_RANGES, type AnalyticsRange } from '../services/analytics';
import {
  ACTIVITY_SECTIONS,
  inDateRange,
  isHistoricalSession,
  isOneShotSession,
  type ActivitySection,
} from '../services/activitySection';
import type { Allowance } from '../types';
type ActivityProps = {
  /** Which section the dropdown shows. Owned by the Focus Sessions
   *  page so the cards and the dropdown stay in sync. */
  section: ActivitySection;
  onSectionChange: (section: ActivitySection) => void;
  /** Open the creation/edit forms in the top Focus Sessions area. */
  onNewSchedule: () => void;
  onNewOneShot: () => void;
  onNewAllowance: () => void;
  onEditAllowance: (allowance: Allowance) => void;
};
export function Activity({
  section,
  onSectionChange,
  onNewSchedule,
  onNewOneShot,
  onNewAllowance,
  onEditAllowance,
}: ActivityProps) {
  const { setup, refresh } = useFocusSetup();
  const [range, setRange] = useState<AnalyticsRange>('all');
  const now = new Date();
  // Historical sessions only: a live session (running, paused,
  // preparing) stays in the active-session area and must never be
  // duplicated here as a historical entry.
  const sessions = useAppStore((s) => s.data.sessions)
    .filter((s) => isHistoricalSession(s) && !isOneShotSession(s))
    .filter((s) => inDateRange(s.started_at, range, now))
    .slice()
    .reverse();
  return (
    <>
      <div className="activity-heading">
        <PageHeading title="Activity" />
      </div>
      <div className="activity-toolbar">
        <div className="activity-section-picker">
          <Dropdown
            value={section}
            options={ACTIVITY_SECTIONS}
            onChange={(value) => onSectionChange(value as ActivitySection)}
          />
        </div>
        {/* The date filter applies to every section, not just the
            session history: one-shots filter on their scheduled
            start, schedules and allowances on their creation date. */}
        <div className="segmented" role="group" aria-label="Date range">
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
      </div>
      {section === 'sessions' ? (
        <div className="session-list">
          {sessions.length ? (
            sessions.map((s) => (
              <SessionCard
                key={s.id}
                session={s}
                showStartedAt
                hideStart
                setup={setup}
                onSetupRefresh={refresh}
              />
            ))
          ) : (
            <Empty>
              No activity in this period. Start a focus session to build your
              history.
            </Empty>
          )}
        </div>
      ) : section === 'schedules' ? (
        <PlanningSections
          only="schedules"
          range={range}
          onNewSchedule={onNewSchedule}
        />
      ) : section === 'oneshots' ? (
        <PlanningSections
          only="oneshots"
          range={range}
          onNewOneShot={onNewOneShot}
          setup={setup}
          onSetupRefresh={refresh}
        />
      ) : (
        <PlanningSections
          only="allowances"
          range={range}
          onNewAllowance={onNewAllowance}
          onEditAllowance={onEditAllowance}
        />
      )}
    </>
  );
}
