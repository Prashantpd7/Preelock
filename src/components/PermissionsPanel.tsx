import { useState } from 'react';
import { useFocusSetup } from '../hooks/useFocusSetup';
import { useAppStore } from '../state/store';
import { api, desktop } from '../services/api';
export function PermissionsPanel() {
  const { setup, error, loading, refresh } = useFocusSetup();
  const [notice, setNotice] = useState('');
  const [reveal, setReveal] = useState(false);
  const browser = useAppStore((s) => s.data.browser) ?? setup?.browser;
  async function requestCamera() {
    try {
      await api.requestCamera();
      setNotice(
        'Respond to the macOS permission prompt, then retry checks. Camera capture starts only in a consented session.',
      );
    } catch (e) {
      setNotice(String(e));
    }
  }
  return (
    <section className="panel form-panel">
      <div className="section-heading">
        <h2>Permissions & focus setup</h2>
        <button
          className="secondary"
          disabled={!desktop || loading}
          onClick={() => void refresh()}
        >
          Retry checks
        </button>
      </div>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {notice && (
        <p role="status" className="notice">
          {notice}
        </p>
      )}
      <div className="capability-row">
        <h3>
          Application redirection ·{' '}
          {setup?.native_available ? 'Available' : 'Unavailable'}
        </h3>
        <p className="muted small">
          macOS hides selected user apps; Windows minimizes their foreground
          windows. This is reversible redirection, not OS application control.
          Finder/Explorer, Settings and system recovery tools remain accessible.
          Only running applications can be selected; open an app then retry
          checks.
        </p>
      </div>
      <div className="capability-row">
        <h3>Activity consent · per session</h3>
        <p className="muted small">
          Opt in during setup to sample the foreground app identifier and idle
          time, plus only the foreground browser hostname when paired. No window
          titles, messages, keystrokes or screen images are collected.
          Accessibility and screen capture are not required by these adapters
          and are not requested.
        </p>
      </div>
      <div className="capability-row">
        <h3>
          Browser filter ·{' '}
          {browser?.connected ? 'Extension connected' : 'Not connected'}
          {browser?.cleanup_pending && ' · cleanup awaiting acknowledgement'}
        </h3>
        <p className="muted small">
          Install the included browser-extension folder via Load unpacked in
          Chrome/Edge Extensions (Developer mode). Pair exactly one browser
          profile using the token below. Reload the extension after upgrading to
          v0.3; accept the tabs permission for hostname sampling. Select that
          same browser during session setup. Incognito, other profiles, other
          browsers, cached pages and other apps are outside this filter.
        </p>
        <p className="muted small">
          Extension folder:{' '}
          <code>
            {setup?.extension_directory ??
              'browser-extension (repository root)'}
          </code>
        </p>
        <p className="muted small">
          No DNS, VPN, proxy or firewall settings are changed. Browser filtering
          still matches URLs with encrypted DNS, VPNs or proxies, but proxy
          websites, extensions and alternate clients can bypass it.
        </p>
        {browser?.error && <p role="status">{browser.error}</p>}
        <div className="row controls">
          <button
            className="secondary"
            disabled={!setup?.pairing_token}
            onClick={() => setReveal(!reveal)}
          >
            {reveal ? 'Hide token' : 'Show pairing token'}
          </button>
          <button
            className="secondary"
            disabled={!desktop}
            onClick={async () => {
              try {
                await api.resetPairing();
                setReveal(false);
                setNotice(
                  'Pairing reset. Reconnect the extension with the new token.',
                );
                await refresh();
              } catch (e) {
                setNotice(String(e));
              }
            }}
          >
            Reset pairing
          </button>
        </div>
        {reveal && (
          <input
            className="pairing-token"
            aria-label="Browser pairing token"
            readOnly
            value={setup?.pairing_token ?? ''}
            onFocus={(e) => e.target.select()}
          />
        )}
      </div>
      <div className="capability-row">
        <h3>
          Camera presence · {setup?.camera_permission ?? 'Check required'}
        </h3>
        <p className="muted small">
          On macOS, Apple Vision estimates whether an upper body is visible,
          locally at about one frame per second. Extended absence begins after
          30 seconds without a detection. No raw frames are saved. This is
          probabilistic presence, not attention or learning verification.
          Phone-use detection and Windows camera analysis are unavailable.
        </p>
        <button
          className="secondary"
          disabled={
            !desktop ||
            setup?.platform !== 'macos' ||
            setup?.camera_permission === 'granted'
          }
          onClick={() => void requestCamera()}
        >
          Request camera permission
        </button>
        <p className="muted small">
          If denied, use macOS System Settings → Privacy & Security → Camera,
          then retry checks. Granting permission alone does not start the
          camera.
        </p>
      </div>
      <div className="capability-row">
        <h3>Deep Focus · unavailable</h3>
        <p className="muted small">
          Requires a separately provisioned restricted account and managed
          application/network policies with administrator recovery. VYSTRIQ does
          not install AppLocker/App Control, Apple MDM payloads or privileged
          Network Extensions. No administrator access is requested.
        </p>
      </div>
      <div className="capability-row">
        <h3>Emergency recovery</h3>
        <p className="muted small">
          Use Emergency exit in the session, type END FOCUS, then confirm. If
          Desktop is unavailable, use the extension popup’s emergency recovery
          or disable/remove the extension. Closing the window keeps the
          controller running. Reopen via the VY menu bar/system tray. Quit and
          release restrictions stops app redirection and camera capture. Browser
          rules expire on the next extension wake/check; browser alarms can be
          delayed. Restarting the browser clears session rules.
        </p>
      </div>
    </section>
  );
}
