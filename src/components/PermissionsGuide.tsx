import { useEffect, useState } from 'react';
import { api, desktop } from '../services/api';
import { settingsAction, type SettingsInfo } from '../services/settings';
import { useFocusSetup } from '../hooks/useFocusSetup';
import { Modal, SettingRow } from './SettingsControls';

export function PermissionsGuide({ onClose }: { onClose: () => void }) {
  const { setup, refresh } = useFocusSetup();
  const [info, setInfo] = useState<SettingsInfo>({});
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [browserGuide, setBrowserGuide] = useState(false);
  const [codeCopied, setCodeCopied] = useState(false);
  const [pathCopied, setPathCopied] = useState(false);
  const mac = setup?.platform === 'macos';
  const browserConnected =
    !!setup?.browser.connected && !setup?.browser.error;
  const folderPath = setup?.extension_directory ?? '';
  // While the guide is open, keep checking so it detects the connection
  // on its own once the user connects in the browser.
  useEffect(() => {
    if (!browserGuide || browserConnected) return;
    const timer = setInterval(() => {
      void refresh();
    }, 3000);
    return () => clearInterval(timer);
  }, [browserGuide, browserConnected, refresh]);
  // Close the guide automatically once the browser connects.
  useEffect(() => {
    if (browserConnected) setBrowserGuide(false);
  }, [browserConnected]);
  async function check() {
    await refresh();
    try {
      setInfo(await settingsAction({ type: 'info' }));
    } catch (e) {
      setError(String(e));
    }
  }
  useEffect(() => {
    let active = true;
    const read = () =>
      void settingsAction({ type: 'info' })
        .then((v) => {
          if (active) setInfo(v);
        })
        .catch((e) => {
          if (active) setError(String(e));
        });
    read();
    window.addEventListener('focus', read);
    return () => {
      active = false;
      window.removeEventListener('focus', read);
    };
  }, []);
  async function act(action: () => Promise<unknown>) {
    setBusy(true);
    setError('');
    try {
      await action();
      await check();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function openExtensions() {
    try {
      // No browser id: opens the extensions page in the default browser.
      await api.openSetup('extensions');
    } catch (e) {
      setError(String(e));
    }
  }
  async function openFolder() {
    try {
      await api.openSetup('extension_folder');
    } catch (e) {
      setError(String(e));
    }
  }
  async function copyCode() {
    const code = setup?.pairing_token ?? '';
    if (!code) return;
    try {
      await navigator.clipboard.writeText(code);
    } catch {
      /* Clipboard unavailable — the code is selectable as text. */
    }
    setCodeCopied(true);
  }
  async function copyFolderPath() {
    if (!folderPath) return;
    try {
      await navigator.clipboard.writeText(folderPath);
    } catch {
      /* Clipboard unavailable — the path is selectable as text. */
    }
    setPathCopied(true);
  }
  const cameraDenied = ['denied', 'restricted'].includes(
    setup?.camera_permission ?? '',
  );
  return (
    <>
      <Modal title="Permissions & setup" onClose={onClose}>
      {!setup ? (
        <p>Checking your system…</p>
      ) : (
        <div className="dialog-stack permission-guide">
          <SettingRow
            title="Camera"
            hint="Camera runs only in sessions where you select it."
          >
            <button
              className="secondary"
              disabled={busy || !mac || setup.camera_permission === 'granted'}
              onClick={() =>
                void act(() =>
                  cameraDenied
                    ? api.openSetup('camera_settings')
                    : api.requestCamera(),
                )
              }
            >
              {!mac
                ? 'Unavailable'
                : setup.camera_permission === 'granted'
                  ? 'Allowed'
                  : cameraDenied
                    ? 'Open settings'
                    : 'Allow camera'}
            </button>
          </SettingRow>
          <SettingRow
            title="Notifications"
            hint={
              mac
                ? 'System Settings → Notifications → Preelock → Allow notifications.'
                : 'Settings → System → Notifications → turn on Preelock. Install the app first; Do not disturb may silence alerts.'
            }
          >
            <button
              className="secondary"
              disabled={busy}
              onClick={() =>
                void act(() =>
                  mac && info.notifications === 0
                    ? settingsAction({ type: 'allow_notifications' })
                    : api.openSetup('notification_settings'),
                )
              }
            >
              {info.notifications === 1
                ? 'Denied · Open settings'
                : mac && info.notifications === 0
                  ? 'Allow notifications'
                  : 'Open settings'}
            </button>
          </SettingRow>
          <SettingRow
            title="Browser extension"
            hint="Connect your browser so Preelock can block distracting sites during sessions."
          >
            <button
              className="secondary"
              disabled={browserConnected}
              onClick={() => setBrowserGuide(true)}
            >
              {browserConnected ? 'Connected' : 'Connect browser'}
            </button>
          </SettingRow>
        </div>
      )}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <div className="dialog-footer">
        <button onClick={onClose}>Done</button>
      </div>
    </Modal>
    {browserGuide && (
      <Modal
        title="Add Preelock to your browser"
        onClose={() => setBrowserGuide(false)}
      >
        <ol className="install-steps">
          <li>
            <strong className="step-title">Open the extensions page</strong>
            <button
              type="button"
              className="secondary"
              onClick={() => void openExtensions()}
            >
              Open extensions page
            </button>
          </li>
          <li>
            <strong className="step-title">Turn on Developer mode</strong>
            <span className="muted">
              The switch at the top-right of the extensions page.
            </span>
          </li>
          <li>
            <strong className="step-title">Load the extension folder</strong>
            {setup?.platform === 'windows' ? (
              <span className="muted">
                Click <strong>Load unpacked</strong> in the browser, paste the
                folder path below into the file picker's address bar, press
                Enter, then select the folder.
              </span>
            ) : (
              <span className="muted">
                Click <strong>Load unpacked</strong> in the browser. In the
                file picker, press <strong>⌘⇧G</strong>, paste the folder
                path, press Enter, then click <strong>Select</strong>.
              </span>
            )}
            <label className="code-label">
              Extension folder on this PC
              <div className="code-row">
                <input
                  aria-label="Extension folder path"
                  readOnly
                  value={folderPath}
                  placeholder="Loading…"
                  onFocus={(e) => e.target.select()}
                />
                <button
                  type="button"
                  className="secondary"
                  disabled={!folderPath}
                  onClick={() => void copyFolderPath()}
                >
                  {pathCopied ? 'Copied' : 'Copy'}
                </button>
              </div>
            </label>
            <p className="muted small">
              Prefer browsing yourself?{' '}
              <button
                type="button"
                className="text-link"
                onClick={() => void openFolder()}
              >
                Open extension folder
              </button>
              .
            </p>
          </li>
          <li>
            <strong className="step-title">Connect the extension</strong>
            <label className="code-label">
              Connection code
              <div className="code-row">
                <input
                  aria-label="Connection code"
                  readOnly
                  value={setup?.pairing_token ?? ''}
                  placeholder="Loading…"
                  onFocus={(e) => e.target.select()}
                />
                <button
                  type="button"
                  className="secondary"
                  disabled={!setup?.pairing_token}
                  onClick={() => void copyCode()}
                >
                  {codeCopied ? 'Copied' : 'Copy code'}
                </button>
              </div>
            </label>
            <span className="muted">
              Click the Preelock icon in your browser's toolbar, paste the
              code, choose your browser, click <strong>Connect</strong> there.
              Keep your browser open.
            </span>
          </li>
        </ol>
        <button type="button" onClick={() => setBrowserGuide(false)}>
          Done
        </button>
      </Modal>
    )}
  </>);
}

export function FirstRunPermissions() {
  const [open, setOpen] = useState(() => {
    try {
      return (
        desktop &&
        localStorage.getItem('preelock-permissions-v10') !== 'seen' &&
        localStorage.getItem('vystriq-permissions-v2') !== 'seen'
      );
    } catch {
      return desktop;
    }
  });
  if (!open) return null;
  return (
    <PermissionsGuide
      onClose={() => {
        try {
          localStorage.setItem('preelock-permissions-v10', 'seen');
        } catch {
          /* Still dismiss for this run. */
        }
        setOpen(false);
      }}
    />
  );
}
