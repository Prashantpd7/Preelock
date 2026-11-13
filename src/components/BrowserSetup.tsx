import { useEffect, useState } from 'react';
import { api } from '../services/api';
import { hasLiveStrictSession } from '../services/activitySection';
import { useAppStore } from '../state/store';
import type { FocusSetup } from '../types';
import { Modal } from './SettingsControls';

export function isSupportedBrowser(id: string): boolean {
  const lower = id.toLowerCase();
  const known = [
    'com.google.chrome',
    'com.microsoft.edgemac',
    'com.brave.browser',
    'com.vivaldi.vivaldi',
    'org.chromium.chromium',
    'ai.perplexity.comet',
  ];
  if (known.includes(lower)) return true;
  return [
    'chrome.exe',
    'msedge.exe',
    'brave.exe',
    'vivaldi.exe',
    'chromium.exe',
    'comet.exe',
  ].some((name) => lower.endsWith(`\\${name}`) || lower === name);
}

export function getBrowserDisplayName(browser: {
  id: string;
  name?: string;
}): string {
  if (browser.name && browser.name.trim()) return browser.name;
  const id = browser.id.toLowerCase();
  if (id.includes('comet')) return 'Comet';
  if (id.includes('brave')) return 'Brave';
  if (id.includes('edgemac') || id.includes('msedge')) return 'Microsoft Edge';
  if (id.includes('vivaldi')) return 'Vivaldi';
  if (id.includes('chromium')) return 'Chromium';
  if (id.includes('chrome')) return 'Google Chrome';
  return browser.id;
}

export function getInstallButtonText(name: string): string {
  const lower = name.toLowerCase();
  if (lower.includes('chrome') && !lower.includes('chromium'))
    return 'Add to Chrome';
  if (lower.includes('brave')) return 'Add to Brave';
  if (lower.includes('edge')) return 'Add to Edge';
  if (lower.includes('comet')) return 'Add to Comet';
  if (lower.includes('vivaldi')) return 'Add to Vivaldi';
  if (lower.includes('chromium')) return 'Add to Chromium';
  return `Add to ${name}`;
}

export function BrowserSetup({
  setup,
  refresh,
  browserId = '',
  allowAdd = false,
  onBack,
}: {
  setup: FocusSetup | null;
  refresh: (browserId?: string) => Promise<unknown>;
  browserId?: string;
  allowAdd?: boolean;
  onBack?: () => void;
}) {
  const [notice, setNotice] = useState('');
  const [copied, setCopied] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [lockedDialog, setLockedDialog] = useState(false);
  const [initialConnectedCount] = useState(
    setup?.connected_browsers?.length ?? 0,
  );

  const strictLive = hasLiveStrictSession(useAppStore((s) => s.data));
  const connected = !!setup?.browser.connected && !setup?.browser.error;

  // Filter only browsers actually installed on the user's computer and supported
  const supportedBrowsers = (setup?.browsers ?? [])
    .filter((b) => isSupportedBrowser(b.id))
    .map((b) => ({
      id: b.id,
      name: getBrowserDisplayName(b),
    }));

  const [explicitBrowserId, setExplicitBrowserId] = useState<string>('');

  // Resolved browser selection:
  // 1. User's explicit choice in dropdown (if valid)
  // 2. browserId prop (if valid)
  // 3. setup.default_browser (if supported & installed)
  // 4. First supported installed browser
  const activeBrowserId = (() => {
    if (
      explicitBrowserId &&
      supportedBrowsers.some(
        (b) => b.id.toLowerCase() === explicitBrowserId.toLowerCase(),
      )
    ) {
      return supportedBrowsers.find(
        (b) => b.id.toLowerCase() === explicitBrowserId.toLowerCase(),
      )!.id;
    }
    if (
      browserId &&
      supportedBrowsers.some(
        (b) => b.id.toLowerCase() === browserId.toLowerCase(),
      )
    ) {
      return supportedBrowsers.find(
        (b) => b.id.toLowerCase() === browserId.toLowerCase(),
      )!.id;
    }
    if (
      setup?.default_browser &&
      supportedBrowsers.some(
        (b) => b.id.toLowerCase() === setup.default_browser!.toLowerCase(),
      )
    ) {
      return supportedBrowsers.find(
        (b) => b.id.toLowerCase() === setup.default_browser!.toLowerCase(),
      )!.id;
    }
    return supportedBrowsers[0]?.id ?? '';
  })();

  // Keep checking while open so it flips to Connected on its own once paired
  useEffect(() => {
    if (connected && !allowAdd) return;
    const timer = setInterval(() => {
      void refresh();
    }, 2000);
    return () => clearInterval(timer);
  }, [connected, allowAdd, refresh]);

  const newlyConnected =
    allowAdd &&
    (setup?.connected_browsers?.length ?? 0) > initialConnectedCount;

  const selectedBrowser =
    supportedBrowsers.find(
      (b) => b.id.toLowerCase() === activeBrowserId.toLowerCase(),
    ) ?? supportedBrowsers[0];

  const installButtonText = selectedBrowser
    ? getInstallButtonText(selectedBrowser.name)
    : 'Add to Chrome';

  async function handleInstallExtension() {
    setNotice('');
    setLaunching(true);
    try {
      const targetBrowser = selectedBrowser?.id ?? activeBrowserId;
      await api.openSetup('install_extension', targetBrowser);
    } catch (e) {
      setNotice(String(e));
    } finally {
      setLaunching(false);
    }
  }

  async function copyCode() {
    const code = setup?.pairing_token ?? '';
    if (!code) return;
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      setNotice('');
      setTimeout(() => setCopied(false), 2000);
    } catch {
      setNotice(
        'Could not copy to clipboard. Please select and copy manually.',
      );
    }
  }

  if (connected && !allowAdd) {
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
              if (strictLive) {
                setLockedDialog(true);
                return;
              }
              void api
                .resetPairing()
                .then(() => refresh())
                .catch((e) => setNotice(String(e)));
            }}
          >
            Disconnect and reconnect
          </button>
        </p>
        {strictLive && (
          <p className="muted small">
            Browser settings are locked during an active focus session.
          </p>
        )}
        {notice && (
          <p role="status" className="small">
            {notice}
          </p>
        )}
        {lockedDialog && (
          <Modal
            title="Browser settings locked"
            onClose={() => setLockedDialog(false)}
          >
            <div className="dialog-stack">
              <p className="muted">
                Browser connection settings cannot be changed during an active
                focus session. You can modify these settings after your current
                session ends.
              </p>
              <button type="button" onClick={() => setLockedDialog(false)}>
                Got it
              </button>
            </div>
          </Modal>
        )}
      </div>
    );
  }

  return (
    <div className="setup-guide">
      {onBack && (
        <button
          type="button"
          className="text-link"
          onClick={onBack}
          style={{ marginBottom: '0.75rem', display: 'inline-block' }}
        >
          ← Back to connected browsers
        </button>
      )}

      {newlyConnected && (
        <div
          style={{
            padding: '0.75rem 1rem',
            borderRadius: '8px',
            backgroundColor: 'rgba(34, 197, 94, 0.15)',
            border: '1px solid rgba(34, 197, 94, 0.3)',
            color: '#4ade80',
            marginBottom: '0.875rem',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            gap: '0.5rem',
          }}
        >
          <span>✓ New browser connected successfully!</span>
          {onBack && (
            <button
              type="button"
              className="secondary"
              onClick={onBack}
              style={{ padding: '0.35rem 0.75rem', fontSize: '0.8rem' }}
            >
              View connected browsers
            </button>
          )}
        </div>
      )}

      {!allowAdd && (
        <p role="status" className="connect-warn">
          Browser is not connected.
        </p>
      )}

      {!allowAdd ? (
        <div style={{ marginTop: '1rem', marginBottom: '1.25rem' }}>
          <strong
            className="step-title"
            style={{
              display: 'block',
              marginBottom: '0.5rem',
              fontSize: '0.95rem',
            }}
          >
            Step 1: Select and install a browser extension
          </strong>
          {supportedBrowsers.length === 0 ? (
            <p className="small muted">
              No supported browser detected. Please install Google Chrome,
              Brave, Microsoft Edge, or Comet.
            </p>
          ) : (
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: '0.625rem',
                flexWrap: 'wrap',
              }}
            >
              <select
                aria-label="Select browser"
                value={selectedBrowser?.id ?? activeBrowserId}
                onChange={(e) => setExplicitBrowserId(e.target.value)}
                style={{
                  minWidth: '140px',
                  maxWidth: '280px',
                  width: 'auto',
                  height: '38px',
                  padding: '6px 32px 6px 12px',
                  background: 'var(--bg)',
                  border: '1px solid var(--border)',
                  borderRadius: '8px',
                  color: 'var(--text)',
                  fontSize: '13px',
                  cursor: 'pointer',
                }}
              >
                {supportedBrowsers.map((b) => (
                  <option key={b.id} value={b.id}>
                    {b.name}
                  </option>
                ))}
              </select>
              <button
                type="button"
                className="secondary"
                disabled={launching || supportedBrowsers.length === 0}
                onClick={() => void handleInstallExtension()}
              >
                {installButtonText}
              </button>
            </div>
          )}
        </div>
      ) : (
        !newlyConnected && (
          <div style={{ marginBottom: '1rem' }}>
            {supportedBrowsers.length === 0 ? (
              <p className="small muted">
                No supported browser detected. Please install Google Chrome,
                Brave, Microsoft Edge, or Comet.
              </p>
            ) : (
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: '0.625rem',
                  flexWrap: 'wrap',
                }}
              >
                <select
                  aria-label="Select browser"
                  value={selectedBrowser?.id ?? activeBrowserId}
                  onChange={(e) => setExplicitBrowserId(e.target.value)}
                  style={{
                    minWidth: '140px',
                    maxWidth: '280px',
                    width: 'auto',
                    height: '38px',
                    padding: '6px 32px 6px 12px',
                    background: 'var(--bg)',
                    border: '1px solid var(--border)',
                    borderRadius: '8px',
                    color: 'var(--text)',
                    fontSize: '13px',
                    cursor: 'pointer',
                  }}
                >
                  {supportedBrowsers.map((b) => (
                    <option key={b.id} value={b.id}>
                      {b.name}
                    </option>
                  ))}
                </select>
                <button
                  type="button"
                  className="secondary"
                  disabled={launching || supportedBrowsers.length === 0}
                  onClick={() => void handleInstallExtension()}
                >
                  {installButtonText}
                </button>
              </div>
            )}
          </div>
        )
      )}

      {(!allowAdd || !newlyConnected) && (
        <div style={{ marginBottom: '0.75rem' }}>
          {!allowAdd && (
            <strong
              className="step-title"
              style={{
                display: 'block',
                marginBottom: '0.5rem',
                fontSize: '0.95rem',
              }}
            >
              Step 2: Connect your browser
            </strong>
          )}
          <label className="code-label" style={{ marginTop: 0 }}>
            Connection code
            <div className="code-row">
              <input
                aria-label="Connection code"
                readOnly
                value={setup?.pairing_token ?? ''}
                placeholder={setup ? 'Generating connection code…' : 'Loading…'}
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
          <p className="muted small code-hint" style={{ marginTop: '0.5rem' }}>
            Copy the code and paste it in the Preelock extension in your
            browser.
          </p>
        </div>
      )}

      {setup?.browser.error && (
        <p role="alert" className="small" style={{ marginTop: '0.5rem' }}>
          {setup.browser.error}
        </p>
      )}
      {notice && (
        <p role="status" className="small" style={{ marginTop: '0.5rem' }}>
          {notice}
        </p>
      )}
    </div>
  );
}
