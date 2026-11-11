import { useState } from 'react';
import { Modal } from './SettingsControls';
import { SessionReview } from './SessionReview';
import { DAY_LABELS, validateScheduleForm } from '../services/schedules';
import { lockLabel } from '../types';
import type { FocusConfig, LockConfig } from '../types';

/** Fields the wizard's schedule popup collects. Passed to the
 *  parent, which creates the schedule with the wizard's FocusConfig
 *  as the snapshot — the same semantics as creating from a template. */
export interface ScheduleCreateFields {
  title: string;
  days: number[];
  start_time: string;
  end_time: string;
  enabled: boolean;
  lock_changes: boolean;
  lock?: LockConfig;
  lock_password?: string;
}

/**
 * End-of-wizard popup for schedule mode: instead of "Start session?",
 * the user sets when the session runs — days of the week, start and
 * end time — plus the same lock options the schedule form has.
 * Cancel returns to the wizard; nothing is created.
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
  const [lockChanges, setLockChanges] = useState(false);
  const [lockPasswordOn, setLockPasswordOn] = useState(false);
  const [lockPassword, setLockPassword] = useState('');
  const [lockChallengeOn, setLockChallengeOn] = useState(false);
  const [lockChallengeLength, setLockChallengeLength] = useState(32);
  const [lockDelayOn, setLockDelayOn] = useState(false);
  const [lockDelayMinutes, setLockDelayMinutes] = useState(5);
  const [lockRestart, setLockRestart] = useState(false);
  const [enabled, setEnabled] = useState(true);
  const [problem, setProblem] = useState('');

  // Days as picked right now — the review below shows them live,
  // and submit() sends exactly these.
  const pickedDays = days.map((on, i) => (on ? i : -1)).filter((i) => i >= 0);
  // Lock summary for the review, in the same words as the details
  // popup. The hash value is never displayed — only whether a
  // password lock is on — so a placeholder stands in for it here.
  const lockText = lockLabel(
    lockPasswordOn || lockChallengeOn || lockDelayOn || lockRestart
      ? {
          password_hash: lockPasswordOn ? 'set' : undefined,
          challenge_length: lockChallengeOn ? lockChallengeLength : 0,
          delay_minutes: lockDelayOn ? lockDelayMinutes : 0,
          restart_lock: lockRestart,
        }
      : undefined,
  );

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
    // Lock checks mirror the schedule form's create path.
    if (lockPasswordOn && !lockPassword.trim()) {
      setProblem('Enter a password for the password lock.');
      return;
    }
    if (
      lockChallengeOn &&
      !(lockChallengeLength >= 8 && lockChallengeLength <= 64)
    ) {
      setProblem('Challenge length must be 8-64 characters.');
      return;
    }
    if (lockDelayOn && !(lockDelayMinutes >= 1 && lockDelayMinutes <= 60)) {
      setProblem('Delay must be 1-60 minutes.');
      return;
    }
    const anyLock =
      lockPasswordOn || lockChallengeOn || lockDelayOn || lockRestart;
    onCreate({
      title: name.trim(),
      days: picked,
      start_time: start,
      end_time: end,
      enabled,
      lock_changes: lockChanges,
      ...(anyLock
        ? {
            lock: {
              password_hash: undefined,
              challenge_length: lockChallengeOn ? lockChallengeLength : 0,
              delay_minutes: lockDelayOn ? lockDelayMinutes : 0,
              restart_lock: lockRestart,
            },
            lock_password:
              lockPasswordOn && lockPassword ? lockPassword : undefined,
          }
        : {}),
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
        </div>
        <p className="small muted">Ends earlier? It runs overnight.</p>
        <label className="check-label">
          <input
            type="checkbox"
            checked={lockChanges}
            onChange={(e) => setLockChanges(e.target.checked)}
          />
          Lock changes during the window
        </label>
        <p className="small muted">
          Locked windows can&rsquo;t be changed early.
        </p>
        <label className="check-label">
          <input
            type="checkbox"
            checked={lockPasswordOn}
            onChange={(e) => setLockPasswordOn(e.target.checked)}
          />
          Lock with a password
        </label>
        {lockPasswordOn && (
          <label>
            Lock password
            <input
              type="password"
              value={lockPassword}
              onChange={(e) => setLockPassword(e.target.value)}
              autoComplete="off"
            />
          </label>
        )}
        <label className="check-label">
          <input
            type="checkbox"
            checked={lockChallengeOn}
            onChange={(e) => setLockChallengeOn(e.target.checked)}
          />
          Lock with a typed challenge
        </label>
        {lockChallengeOn && (
          <label>
            Challenge length (characters, 8-64)
            <input
              type="number"
              min={8}
              max={64}
              value={lockChallengeLength}
              onChange={(e) => setLockChallengeLength(Number(e.target.value))}
            />
          </label>
        )}
        <label className="check-label">
          <input
            type="checkbox"
            checked={lockDelayOn}
            onChange={(e) => setLockDelayOn(e.target.checked)}
          />
          Lock with a delay
        </label>
        {lockDelayOn && (
          <label>
            Delay (minutes, 1-60)
            <input
              type="number"
              min={1}
              max={60}
              value={lockDelayMinutes}
              onChange={(e) => setLockDelayMinutes(Number(e.target.value))}
            />
          </label>
        )}
        <label className="check-label">
          <input
            type="checkbox"
            checked={lockRestart}
            onChange={(e) => setLockRestart(e.target.checked)}
          />
          Keep the lock after a restart
        </label>
        {(lockPasswordOn || lockChallengeOn || lockDelayOn) && (
          <p className="small muted">Locks pass together, lift at end.</p>
        )}
        <label className="check-label">
          <input
            type="checkbox"
            checked={enabled}
            onChange={(e) => setEnabled(e.target.checked)}
          />
          Enabled
        </label>
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
            lockText={lockText}
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
