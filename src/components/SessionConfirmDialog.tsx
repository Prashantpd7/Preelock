import { useState } from 'react';
import { Modal } from './SettingsControls';
import { SessionReview } from './SessionReview';
import { sessionNeedsExtension } from '../services/browserSupport';
import type { FocusConfig } from '../types';

/**
 * Confirmation shown after the session wizard finishes. When the
 * wizard's FocusConfig is passed (the normal path), the popup is
 * a full review of the session about to start — overview,
 * monitoring, apps, websites, YouTube and browser readiness —
 * read from the exact config that will be submitted. Without a
 * config it falls back to the old compact summary.
 * Strict sessions get an extra one-line warning. Cancel discards
 * the wizard state — nothing is saved.
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
  onConnectBrowser,
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
  onConnectBrowser?: () => void;
}) {
  const [askConnect, setAskConnect] = useState(false);
  const needsExtension = focus ? sessionNeedsExtension(focus) : false;
  const missingBrowser = needsExtension && !browserConnected;

  function handleStartClick() {
    if (missingBrowser && !askConnect) {
      setAskConnect(true);
      return;
    }
    onStart();
  }

  return (
    <Modal
      title={
        askConnect ? 'Connect browser before starting?' : 'Review your session'
      }
      onClose={onCancel}
    >
      <div className="dialog-stack">
        {askConnect ? (
          <div>
            <p>
              <strong>Browser extension is not connected.</strong>
            </p>
            <p className="muted" style={{ margin: '0.75rem 0' }}>
              This session includes website rules, but Preelock is not connected
              to your browser. Distracting websites will stay unblocked until
              you connect the extension.
            </p>
            <div
              className="row controls"
              style={{ justifyContent: 'flex-end', marginTop: '1rem' }}
            >
              <button
                type="button"
                className="secondary"
                onClick={() => setAskConnect(false)}
                disabled={busy}
              >
                Back
              </button>
              {onConnectBrowser && (
                <button
                  type="button"
                  className="secondary"
                  onClick={onConnectBrowser}
                  disabled={busy}
                >
                  Connect browser
                </button>
              )}
              <button type="button" onClick={onStart} disabled={busy}>
                Start anyway
              </button>
            </div>
          </div>
        ) : (
          <>
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
            <div
              className="row controls"
              style={{ justifyContent: 'flex-end' }}
            >
              <button
                type="button"
                className="secondary"
                onClick={onCancel}
                disabled={busy}
              >
                Cancel
              </button>
              <button type="button" onClick={handleStartClick} disabled={busy}>
                Start session
              </button>
            </div>
          </>
        )}
      </div>
    </Modal>
  );
}
