import { useEffect, useState } from 'react';
import { api } from '../services/api';
import type { FocusSetup } from '../types';
import { Modal } from './SettingsControls';
export function BrowserSetup({
  setup,
  refresh,
  browserId = '',
}: {
  setup: FocusSetup | null;
  refresh: () => Promise<void>;
  browserId?: string;
}) {
  const [notice, setNotice] = useState('');
  const [copied, setCopied] = useState(false);
  const [installGuide, setInstallGuide] = useState(false);
  const [pathCopied, setPathCopied] = useState(false);
  // Detected live on this PC by the app — never a hardcoded path.
  const folderPath = setup?.extension_directory ?? '';
  // The app already filters the connection status to this browser:
  // connected here means the paired extension lives in this browser.
  const connected = !!setup?.browser.connected && !setup?.browser.error;
  // Keep checking while the guide is open so it flips to Connected
  // on its own once the user connects in the browser.
  useEffect(() => {
    if (connected) return;
    const timer = setInterval(() => {
      void refresh();
    }, 3000);
    return () => clearInterval(timer);
  }, [connected, refresh]);
  // When connected, show a confirmation instead of rendering nothing —
  // returning null here left an empty broken dialog in the "Before you
  // start" popup when the fresh status check already showed connected.
  if (connected) {
    return (
      <div className="setup-guide">
        <p className="ready-box">Connected</p>
        <p className="muted small">
          The browser extension is paired and reporting. You can start the
          session.
        </p>
        <p className="muted small">
          Wrong browser?{' '}
          <button
            type="button"
            className="text-link"
            onClick={() => {
              void api
                .resetPairing()
                .then(() => refresh())
                .catch((e) => setNotice(String(e)));
            }}
          >
            Disconnect and reconnect
          </button>
        </p>
        {notice && (
          <p role="status" className="small">
            {notice}
          </p>
        )}
      </div>
    );
  }
  async function openExtensions() {
    try {
      // No browser id: opens the extensions page in the default browser.
      await api.openSetup('extensions', browserId);
      setNotice('');
    } catch (e) {
      setNotice(String(e));
    }
  }
  async function openFolder() {
    try {
      await api.openSetup('extension_folder', browserId);
      setNotice('');
    } catch (e) {
      setNotice(String(e));
    }
  }
  async function copyFolderPath() {
    if (!folderPath) return;
    try {
      await navigator.clipboard.writeText(folderPath);
    } catch {
      // Clipboard unavailable — the path is shown as text below.
    }
    setPathCopied(true);
    setNotice('');
  }
  async function copyCode() {
    const code = setup?.pairing_token ?? '';
    if (!code) return;
    try {
      await navigator.clipboard.writeText(code);
    } catch {
      // Clipboard unavailable — the code is selectable as text below.
    }
    setCopied(true);
    setNotice('');
  }
  return (
    <div className="setup-guide">
      <p role="alert" className="connect-warn">
        Browser is not connected. Follow the steps below to connect it.
      </p>
      <div className="row">
        <button
          type="button"
          className="secondary"
          onClick={() => void openExtensions()}
        >
          Open Extensions
        </button>
      </div>
      <label className="code-label">
        Connection code
        <div className="code-row">
          <input
            aria-label="Connection code"
            readOnly
            value={setup?.pairing_token ?? ''}
            onFocus={(e) => e.target.select()}
          />
          <button
            type="button"
            className="secondary"
            disabled={!setup?.pairing_token}
            onClick={() => void copyCode()}
          >
            {copied ? 'Copied' : 'Copy code'}
          </button>
        </div>
      </label>
      <p className="muted small code-hint">
        Copy the code and paste it in the Preelock extension in your browser
        toolbar, then click Connect there.
      </p>
      <p className="muted small">
        Extension not installed?{' '}
        <button
          type="button"
          className="text-link"
          onClick={() => setInstallGuide(true)}
        >
          Click here
        </button>{' '}
        to see how to add it.
      </p>
      {setup?.browser.error && (
        <p role="alert" className="small">
          {setup.browser.error}
        </p>
      )}
      {notice && (
        <p role="status" className="small">
          {notice}
        </p>
      )}
      {installGuide && (
        <Modal
          title="Add Preelock to your browser"
          onClose={() => setInstallGuide(false)}
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
              <span className="muted">
                Copy the <strong>connection code</strong> from the setup guide,
                click the Preelock icon in your browser's toolbar, paste the
                code, and click <strong>Connect</strong> there. Keep your
                browser open.
              </span>
            </li>
          </ol>
          <button type="button" onClick={() => setInstallGuide(false)}>
            Done
          </button>
        </Modal>
      )}
    </div>
  );
}
