import { useEffect, useState } from 'react';
import { api } from '../services/api';
import { hasLiveStrictSession } from '../services/activitySection';
import { useAppStore } from '../state/store';
import { isSafariId } from '../services/browserSupport';
import { sameBrowser, type FocusSetup } from '../types';
import { Modal } from './SettingsControls';

export function isSupportedBrowser(id: string): boolean {
  const lower = id.toLowerCase();
  if (lower === 'com.apple.safari' || lower.endsWith('\\safari.exe'))
    return true;
  const known = [
    'com.google.chrome',
    'com.microsoft.edgemac',
    'com.brave.browser',
    'com.vivaldi.vivaldi',
    'org.chromium.chromium',
    'ai.perplexity.comet',
    'company.thebrowser.browser',
    'com.operasoftware.opera',
    'com.operasoftware.operagx',
    'com.apple.safari',
  ];
  if (known.includes(lower)) return true;
  return [
    'chrome.exe',
    'msedge.exe',
    'brave.exe',
    'vivaldi.exe',
    'chromium.exe',
    'comet.exe',
    'arc.exe',
    'opera.exe',
    'safari.exe',
  ].some((name) => lower.endsWith(`\\${name}`) || lower === name);
}

export function getCanonicalBrowserKey(id: string): string {
  const lower = id.toLowerCase();
  if (lower.includes('safari')) return 'safari';
  if (lower.includes('chrome') && !lower.includes('chromium')) return 'chrome';
  if (lower.includes('brave')) return 'brave';
  if (
    lower.includes('edgemac') ||
    lower.includes('msedge') ||
    lower.includes('edge')
  )
    return 'edge';
  if (lower.includes('comet')) return 'comet';
  if (lower.includes('vivaldi')) return 'vivaldi';
  if (lower.includes('chromium')) return 'chromium';
  if (lower.includes('arc') || lower.includes('thebrowser')) return 'arc';
  if (lower.includes('operagx')) return 'operagx';
  if (lower.includes('opera')) return 'opera';
  return lower;
}

export function isBrowserOpen(
  browserId: string,
  setup: FocusSetup | null,
): boolean {
  if (!setup?.applications) return false;
  const key = getCanonicalBrowserKey(browserId);
  return setup.applications.some(
    (app) =>
      sameBrowser(app.id, browserId) ||
      getCanonicalBrowserKey(app.id) === key ||
      (app.name && getCanonicalBrowserKey(app.name) === key),
  );
}

try {
  localStorage.removeItem('preelock-known-connected-browsers');
  localStorage.removeItem('preelock-all-browsers-connected');
} catch {
  // ignore
}

export function markBrowserConnected(_browserId?: string): void {
  void _browserId;
  // Connection state is derived directly from live extension status, not cached locally
}

export function getKnownConnectedBrowsers(): string[] {
  return [];
}

export function isBrowserKnownConnected(_browserId?: string): boolean {
  void _browserId;
  return false;
}

export function isBrowserConnected(
  browserId: string,
  setup: FocusSetup | null,
): boolean {
  if (isSafariId(browserId)) return false;
  if (!setup) return false;
  const key = getCanonicalBrowserKey(browserId);
  const connectedList = setup.connected_browsers ?? [];
  if (
    connectedList.some(
      (c) => sameBrowser(c, browserId) || getCanonicalBrowserKey(c) === key,
    )
  ) {
    return true;
  }
  if (
    setup.browser?.connected &&
    !setup.browser?.error &&
    setup.browser?.client_browser
  ) {
    if (
      sameBrowser(setup.browser.client_browser, browserId) ||
      getCanonicalBrowserKey(setup.browser.client_browser) === key
    ) {
      return true;
    }
  }
  return false;
}

export function getInstalledSupportedBrowsers(
  setup: FocusSetup | null,
): { id: string; name: string }[] {
  if (!setup) return [];
  const mac = setup.platform === 'macos';
  const raw = (setup.browsers ?? []).filter(
    (b) =>
      isSupportedBrowser(b.id) &&
      !(
        mac &&
        (b.id.toLowerCase().includes('safari') || b.id === 'com.apple.Safari')
      ),
  );
  const seen = new Set<string>();
  const installed: { id: string; name: string }[] = [];
  for (const b of raw) {
    const key = getCanonicalBrowserKey(b.id);
    if (!seen.has(key)) {
      seen.add(key);
      installed.push({ id: b.id, name: getBrowserDisplayName(b) });
    }
  }
  return installed;
}

export function getUnconnectedInstalledBrowsers(
  setup: FocusSetup | null,
): { id: string; name: string }[] {
  if (!setup) return [];
  const installed = getInstalledSupportedBrowsers(setup);
  return installed.filter((b) => !isBrowserConnected(b.id, setup));
}

export function getOpenSupportedBrowsers(
  setup: FocusSetup | null,
): { id: string; name: string }[] {
  const installed = getInstalledSupportedBrowsers(setup);
  return installed.filter((b) => isBrowserOpen(b.id, setup));
}

export function getUnconnectedOpenBrowsers(
  setup: FocusSetup | null,
): { id: string; name: string }[] {
  const openBrowsers = getOpenSupportedBrowsers(setup);
  return openBrowsers.filter((b) => !isBrowserConnected(b.id, setup));
}

export function areAllInstalledBrowsersConnected(
  setup: FocusSetup | null,
): boolean {
  if (!setup) return false;
  const installed = getInstalledSupportedBrowsers(setup);
  if (installed.length === 0) {
    return !!setup.browser.connected && !setup.browser.error;
  }
  const unconnected = getUnconnectedInstalledBrowsers(setup);
  return unconnected.length === 0;
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
  if (id.includes('arc') || id.includes('thebrowser')) return 'Arc';
  if (id.includes('operagx')) return 'Opera GX';
  if (id.includes('opera')) return 'Opera';
  if (id.includes('safari')) return 'Safari';
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
  if (lower.includes('arc')) return 'Add to Arc';
  if (lower.includes('opera')) return 'Add to Opera';
  return `Add to ${name}`;
}

export function BrowserSetup({
  setup,
  refresh,
  browserId = '',
  allowAdd = false,
  onBack,
  singleBrowserId,
}: {
  setup: FocusSetup | null;
  refresh: (browserId?: string) => Promise<unknown>;
  browserId?: string;
  allowAdd?: boolean;
  onBack?: () => void;
  singleBrowserId?: string;
}) {
  const [notice, setNotice] = useState('');
  const [copied, setCopied] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [lockedDialog, setLockedDialog] = useState(false);
  const [initialConnectedCount] = useState(
    setup?.connected_browsers?.length ?? 0,
  );

  const strictLive = hasLiveStrictSession(useAppStore((s) => s.data));
  const rawInstalledBrowsers = getInstalledSupportedBrowsers(setup);
  const installedBrowsers = singleBrowserId
    ? (() => {
        const found = rawInstalledBrowsers.filter(
          (b) =>
            sameBrowser(b.id, singleBrowserId) ||
            getCanonicalBrowserKey(b.id) ===
              getCanonicalBrowserKey(singleBrowserId),
        );
        if (found.length > 0) return found;
        const fallback = (setup?.browsers ?? []).find(
          (b) =>
            sameBrowser(b.id, singleBrowserId) ||
            getCanonicalBrowserKey(b.id) ===
              getCanonicalBrowserKey(singleBrowserId),
        );
        return fallback
          ? [{ id: fallback.id, name: getBrowserDisplayName(fallback) }]
          : rawInstalledBrowsers;
      })()
    : rawInstalledBrowsers;
  const unconnectedBrowsers = getUnconnectedInstalledBrowsers(setup);
  const allConnected = singleBrowserId
    ? isBrowserConnected(singleBrowserId, setup)
    : areAllInstalledBrowsersConnected(setup);
  const connected = allConnected;

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
    if (allConnected && !allowAdd) return;
    const timer = setInterval(() => {
      void refresh();
    }, 2000);
    return () => clearInterval(timer);
  }, [allConnected, allowAdd, refresh]);

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

  async function handleInstallBrowser(id: string) {
    setNotice('');
    setLaunching(true);
    try {
      await api.openSetup('install_extension', id);
    } catch (e) {
      setNotice(String(e));
    } finally {
      setLaunching(false);
    }
  }

  async function handleInstallExtension() {
    const targetBrowser = selectedBrowser?.id ?? activeBrowserId;
    await handleInstallBrowser(targetBrowser);
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

  if (connected && !allowAdd && !singleBrowserId) {
    return (
      <div className="setup-guide">
        <p className="ready-box">All browsers connected</p>
        <p className="muted small">
          All installed browsers are paired and reporting. Close this window and
          click Done to continue.
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

      {!allowAdd ? (
        <div style={{ marginTop: '0.5rem', marginBottom: '1.25rem' }}>
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
            installedBrowsers.length > 0 && (
              <div
                style={{
                  display: 'flex',
                  flexDirection: 'column',
                  gap: '0.5rem',
                }}
              >
                <span className="small muted" style={{ fontWeight: 600 }}>
                  {singleBrowserId
                    ? `${installedBrowsers[0]?.name ?? 'Browser'} extension:`
                    : `Installed browsers (${installedBrowsers.length - unconnectedBrowsers.length}/${installedBrowsers.length} connected):`}
                </span>
                {installedBrowsers.map((b) => {
                  const isConn = isBrowserConnected(b.id, setup);
                  return (
                    <div
                      key={b.id}
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        justifyContent: 'space-between',
                        padding: '7px 12px',
                        borderRadius: '6px',
                        background: 'var(--panel)',
                        border: isConn
                          ? '1px solid rgba(34, 197, 94, 0.3)'
                          : '1px solid var(--border)',
                      }}
                    >
                      <span style={{ fontSize: '13px', fontWeight: 500 }}>
                        {b.name}
                      </span>
                      {isConn ? (
                        <span
                          style={{
                            fontSize: '12px',
                            color: '#4ade80',
                            fontWeight: 600,
                          }}
                        >
                          ✓ Connected
                        </span>
                      ) : (
                        <button
                          type="button"
                          className="secondary"
                          style={{ padding: '3px 8px', fontSize: '12px' }}
                          disabled={launching}
                          aria-label={getInstallButtonText(b.name)}
                          onClick={() => void handleInstallBrowser(b.id)}
                        >
                          Install
                        </button>
                      )}
                    </div>
                  );
                })}
              </div>
            )
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
