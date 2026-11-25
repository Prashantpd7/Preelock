import { Modal } from './SettingsControls';
import { SessionReview } from './SessionReview';
import type { FocusConfig } from '../types';

/**
 * Confirmation shown after the session wizard finishes. When the
 * wizard's FocusConfig is passed (the normal path), the popup is
 * a full review of the session about to start — overview,
 * monitoring, apps, websites, YouTube and browser readiness —
 * read from the exact config that will be submitted. Without a
 * config it falls back to the old compact summary.
 * Strict sessions get an extra one-line warning. Closing the
 * popup only dismisses it — the wizard state is kept, so the
 * user returns to the filled-in setup. Starting does not
 * require the browser extension: the DNS proxy blocks websites,
 * and the extension (when connected) only shows the block page
 * and handles YouTube content rules.
 */
export function SessionConfirmDialog({
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
  onStart,
  onCancel,
}: {
  title: string;
  minutes: number;
  strict: boolean;
  busy: boolean;
  focus?: FocusConfig;
  appNames?: Record<string, string>;
  browserName?: string;
  browserConnected?: boolean;
  cameraPermission?: string;
  platform?: string;
  safariWarning?: boolean;
  onStart: () => void;
  onCancel: () => void;
}) {
  return (
    <Modal title="Review your session" onClose={onCancel}>
      <div className="dialog-stack">
        {focus ? (
          <SessionReview
            focus={focus}
            title={title}
            timing={{ kind: 'immediate', minutes }}
            appNames={appNames}
            browserName={browserName}
            browserConnected={browserConnected}
            cameraPermission={cameraPermission}
            platform={platform}
            safariWarning={safariWarning}
          />
        ) : (
          <div>
            <p>
              <strong>{title}</strong>
            </p>
            <p className="muted">
              {minutes} min · {strict ? 'Strict' : 'Standard'} mode
            </p>
          </div>
        )}
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
