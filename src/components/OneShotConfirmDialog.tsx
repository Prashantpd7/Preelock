import { useState } from 'react';
import { Modal } from './SettingsControls';
import { SessionReview } from './SessionReview';
import { oneShotFireAtMs } from '../services/one_shots';
import type { FocusConfig } from '../types';

/**
 * End-of-wizard popup for one-shot mode: instead of "Start session?",
 * the user picks the date and time the session should start
 * automatically. Cancel returns to the wizard; nothing is created.
 */
export function OneShotConfirmDialog({
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
  onCreate: (fields: { title: string; fire_at_ms: number }) => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState(title);
  const [when, setWhen] = useState('');
  const [problem, setProblem] = useState('');

  function submit() {
    // Same checks as the one-shot form, minus the template (the
    // wizard's session is the template here).
    if (!name.trim() || name.trim().length > 160) {
      setProblem('Use a title between 1 and 160 characters.');
      return;
    }
    const fireAtMs = oneShotFireAtMs(when);
    if (fireAtMs === null) {
      setProblem('Pick a date and time.');
      return;
    }
    if (fireAtMs <= Date.now()) {
      setProblem('Pick a future date and time.');
      return;
    }
    onCreate({ title: name.trim(), fire_at_ms: fireAtMs });
  }

  return (
    <Modal title="Start once at" onClose={onCancel}>
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
          One-shot name
          <input
            required
            maxLength={160}
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="e.g. Evening deep work"
          />
        </label>
        <label>
          Start at
          <input
            type="datetime-local"
            required
            value={when}
            onChange={(e) => setWhen(e.target.value)}
          />
        </label>
        {focus && (
          <SessionReview
            focus={focus}
            title={name}
            timing={{
              kind: 'oneshot',
              minutes,
              fireAtMs: oneShotFireAtMs(when),
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
            Create one-shot
          </button>
        </div>
      </div>
    </Modal>
  );
}
