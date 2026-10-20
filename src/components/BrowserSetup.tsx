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
  const [checking, setChecking] = useState(false);
  const [pathCopied, setPathCopied] = useState(false);
  // Detected live on this PC by the app — never a hardcoded path.
  const folderPath = setup?.extension_directory ?? '';
  const browser = browserId || setup?.browsers?.[0]?.id || '';
  // The app already filters the connection status to this browser:
  // connected here means the paired extension lives in this browser.
  const connected = !!setup?.browser.connected && !setup?.browser.error;
  const browserName =
    setup?.browsers?.find((b) => b.id === browser)?.name || 'This browser';
  // Keep checking while the guide is open so it flips to Connected
  // on its own once the user connects in the browser.
  useEffect(() => {
    if (connected) return;
    const timer = setInterval(() => {
      void refresh();
    }, 3000);
    return () => clearInterval(timer);
  }, [connected, refresh]);
  if (connected) return null;
  async function openExtensions() {
    try {
      await api.openSetup('extensions', browser);
      setNotice('');
    } catch (e) {
      setNotice(String(e));
    }
  }
  async function openFolder() {
    try {
      await api.openSetup('extension_folder');
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
  async function checkNow() {
    setChecking(true);
    setNotice('');
    // Keep re-checking for a few seconds: the moment the selected
    // browser's extension syncs, this guide hides and shows Connected.
    for (let i = 0; i < 5; i += 1) {
      await refresh();
      await new Promise((r) => setTimeout(r, 2000));
    }
    setChecking(false);
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
      {browser && (
        <p role="alert" className="connect-warn">
          {browserName} is not connected. Follow the steps below to connect{' '}
          {browserName}.
        </p>
      )}
      <div className="row">
        <button
          type="button"
          className="secondary"
          disabled={!browser}
          onClick={() => void openExtensions()}
        >
          Open Extensions
        </button>
        <button
          type="button"
          className="secondary"
          disabled={!browser || checking}
          onClick={() => void checkNow()}
        >
          {checking ? 'Checking…' : 'Connect'}
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
        Copy the code and paste it in the Vystriq extension in your browser
        toolbar, choose {browserName}, then click Connect there.
      </p>
      <p className="muted small">
        Extension not installed in this browser?{' '}
        <button
          type="button"
          className="text-link"
          disabled={!browser}
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
          title={`Add Vystriq to ${browserName}`}
          onClose={() => setInstallGuide(false)}
        >
          <ol className="install-steps">
            <li>
              <strong className="step-title">Open the extensions page</strong>
              <button
                type="button"
                className="secondary"
                disabled={!browser}
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
                click the Vystriq icon in {browserName}'s toolbar, paste the
                code, choose {browserName}, and click <strong>Connect</strong>{' '}
                there. Keep {browserName} open.
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
