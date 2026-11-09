import { Modal } from './SettingsControls';

/**
 * Confirmation shown after the session wizard finishes: a compact
 * summary of the session about to start, with "Start session" and
 * "Cancel". Cancel discards the wizard state — nothing is saved.
 * Strict sessions get an extra one-line warning.
 */
export function SessionConfirmDialog({
  title,
  minutes,
  strict,
  busy,
  onStart,
  onCancel,
}: {
  title: string;
  minutes: number;
  strict: boolean;
  busy: boolean;
  onStart: () => void;
  onCancel: () => void;
}) {
  return (
    <Modal title="Start session?" onClose={onCancel}>
      <div className="dialog-stack">
        <div>
          <p>
            <strong>{title}</strong>
          </p>
          <p className="muted">
            {minutes} min · {strict ? 'Strict' : 'Standard'} mode
          </p>
        </div>
        {strict && (
          <p>
            Strict mode will block everything except your work apps. This
            can&rsquo;t be undone — finish the session properly.
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
          <button type="button" onClick={onStart} disabled={busy}>
            Start session
          </button>
        </div>
      </div>
    </Modal>
  );
}
