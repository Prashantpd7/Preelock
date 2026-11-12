import { useState } from 'react';
import { Modal } from './SettingsControls';
import { SessionReview } from './SessionReview';
import {
  DAY_LABELS,
  scheduleTotalTime,
  validateScheduleForm,
} from '../services/schedules';
import type { FocusConfig } from '../types';

/** Fields the wizard's schedule popup collects. Passed to the
 *  parent, which creates the schedule with the wizard's FocusConfig
 *  as the snapshot — the same semantics as creating from a template. */
export interface ScheduleCreateFields {
  title: string;
  days: number[];
  start_time: string;
  end_time: string;
  enabled: boolean;
  /** Always false now: the popup offers no lock options, so
   *  schedules created here carry no lock. */
  lock_changes: boolean;
}

/**
 * End-of-wizard popup for schedule mode: instead of "Start session?",
 * the user sets when the session runs — days of the week, start and
 * end time. The popup offers no lock options: schedules created
 * here carry no lock. Cancel returns to the wizard; nothing is created.
 */
export function ScheduleConfirmDialog({
  title,
  minutes,
  strict,
  busy,
  focus,
  appNames,
  browserName,
  browserConnected,
  cameraPermission,
  platform,
  safariWarning,
  onCreate,
  onCancel,
}: {
  title: string;
  minutes: number;
  strict: boolean;
  busy: boolean;
  /** The wizard's FocusConfig: the review reads it directly. */
  focus?: FocusConfig;
  appNames?: Record<string, string>;
  browserName?: string;
  browserConnected?: boolean;
  cameraPermission?: string;
  platform?: string;
  safariWarning?: boolean;
  onCreate: (fields: ScheduleCreateFields) => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState(title);
  const [days, setDays] = useState<boolean[]>([
    false,
    true,
    true,
    true,
    true,
    true,
    false,
  ]);
  const [start, setStart] = useState('09:00');
  const [end, setEnd] = useState('10:00');
  const [problem, setProblem] = useState('');

  // Days as picked right now — the review below shows them live,
  // and submit() sends exactly these.
  const pickedDays = days.map((on, i) => (on ? i : -1)).filter((i) => i >= 0);

  function submit() {
    const picked = pickedDays;
    const formProblem = validateScheduleForm({
      title: name,
      days: picked,
      start_time: start,
      end_time: end,
    });
    if (formProblem) {
      setProblem(formProblem);
      return;
    }
    onCreate({
      title: name.trim(),
      days: picked,
      start_time: start,
      end_time: end,
      enabled: true,
      lock_changes: false,
    });
  }

  return (
    <Modal title="Schedule session" onClose={onCancel}>
      <div className="dialog-stack">
        <p className="muted">
          {minutes} min · {strict ? 'Strict' : 'Standard'} mode
        </p>
        {strict && (
          <p>
            Strict mode will block everything except your work apps. This
            can&rsquo;t be undone — finish the session properly.
          </p>
        )}
        <label className="grow">
          Schedule name
          <input
            required
            maxLength={160}
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="e.g. Morning deep work"
          />
        </label>
        <fieldset>
          <legend>Days</legend>
          <div className="row controls">
            {DAY_LABELS.map((label, i) => (
              <label className="check-label" key={label}>
                <input
                  type="checkbox"
                  checked={days[i]}
                  onChange={() =>
                    setDays((all) => all.map((on, j) => (j === i ? !on : on)))
                  }
                />
                {label}
              </label>
            ))}
          </div>
        </fieldset>
        <div className="simple-grid time-grid">
          <label>
            Start time
            <input
              type="time"
              required
              value={start}
              onChange={(e) => setStart(e.target.value)}
            />
          </label>
          <label>
            End time
            <input
              type="time"
              required
              value={end}
              onChange={(e) => setEnd(e.target.value)}
            />
          </label>
          <label>
            Total time
            <input
              type="text"
              readOnly
              tabIndex={-1}
              value={scheduleTotalTime(start, end)}
            />
          </label>
        </div>
        {focus && (
          <SessionReview
            focus={focus}
            title={name}
            timing={{
              kind: 'schedule',
              minutes,
              days: pickedDays,
              startTime: start,
              endTime: end,
            }}
            appNames={appNames}
            browserName={browserName}
            browserConnected={browserConnected}
            cameraPermission={cameraPermission}
            platform={platform}
            safariWarning={safariWarning}
          />
        )}
        {problem && (
          <p className="error small" role="alert">
            {problem}
          </p>
        )}
        <div className="row controls" style={{ justifyContent: 'flex-end' }}>
          <button
            type="button"
            className="secondary"
            onClick={onCancel}
            disabled={busy}
          >
            Cancel
          </button>
          <button type="button" onClick={submit} disabled={busy}>
            Create schedule
          </button>
        </div>
      </div>
    </Modal>
  );
}
