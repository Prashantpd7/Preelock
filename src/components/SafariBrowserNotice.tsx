import { useState } from 'react';
import { api } from '../services/api';
import { SUPPORTED_DOWNLOADS } from '../services/browserSupport';
import { sameBrowser, type FocusSetup } from '../types';
import { Modal } from './SettingsControls';

/**
 * Session-only compatibility notice: Safari has no Preelock extension, so
 * it cannot enforce a session's website rules. Shown from the session
 * wizard (manual, schedule, one-shot, editing) and nowhere else — normal
 * Safari use and screen-time tracking never see it.
 *
 * Choosing a connected browser swaps Safari out for it in place; choosing
 * an installed but disconnected one hands over to the existing Connect
 * browser guide, unchanged.
 */
export function SafariBrowserNotice({
  setup,
  onChoose,
  onCancel,
  onRefresh,
}: {
  setup: FocusSetup | null;
  onChoose: (browserId: string) => void;
  onCancel: () => void;
  /** Re-reads the installed-browser list after the user installs one. */
  onRefresh?: () => Promise<void> | void;
}) {
  const [notice, setNotice] = useState('');
  const browsers = setup?.browsers ?? [];
  const connected = setup?.connected_browsers ?? [];
  const isConnected = (id: string) => connected.some((c) => sameBrowser(c, id));

  async function openDownload(id: string) {
    try {
      setNotice('');
      await api.openSetup('browser_download', id);
    } catch (e) {
      setNotice(String(e));
    }
  }

  if (!browsers.length) {
    return (
      <Modal title="No supported browser found" onClose={onCancel}>
        <div className="dialog-stack">
          <p>
            Install a supported browser to enable website blocking during focus
            sessions.
          </p>
          <div className="row controls">
            {SUPPORTED_DOWNLOADS.map((b) => (
              <button
                key={b.id}
                type="button"
                className="secondary"
                onClick={() => void openDownload(b.id)}
              >
                {b.name}
              </button>
            ))}
          </div>
          {notice && (
            <p role="alert" className="error small">
              {notice}
            </p>
          )}
          <div className="row controls" style={{ justifyContent: 'flex-end' }}>
            <button type="button" className="secondary" onClick={onCancel}>
              Cancel
            </button>
            {onRefresh && (
              <button
                type="button"
                className="secondary"
                onClick={() => void onRefresh()}
              >
                Refresh
              </button>
            )}
            <button
              type="button"
              onClick={() => void openDownload('com.google.Chrome')}
            >
              Get a browser
            </button>
          </div>
        </div>
      </Modal>
    );
  }

  const preferred = browsers.find((b) => isConnected(b.id)) ?? browsers[0];
  return (
    <Modal title="Safari isn't supported yet" onClose={onCancel}>
      <div className="dialog-stack">
        <p>
          Preelock needs a supported browser to block websites during your
          session.
        </p>
        <div className="dialog-stack">
          {browsers.map((b) => (
            <button
              key={b.id}
              type="button"
              className="secondary"
              onClick={() => onChoose(b.id)}
            >
              {b.name} · {isConnected(b.id) ? 'Connected' : 'Not connected'}
            </button>
          ))}
        </div>
        <div className="row controls" style={{ justifyContent: 'flex-end' }}>
          <button type="button" className="secondary" onClick={onCancel}>
            Cancel
          </button>
          <button type="button" onClick={() => onChoose(preferred.id)}>
            Choose browser
          </button>
        </div>
      </div>
    </Modal>
  );
}
