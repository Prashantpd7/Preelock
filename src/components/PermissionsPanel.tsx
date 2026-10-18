import { useState } from 'react';
import { useFocusSetup } from '../hooks/useFocusSetup';
import { api, desktop } from '../services/api';
import { BrowserSetup } from './BrowserSetup';
export function PermissionsPanel() {
  const { setup, error, loading, refresh } = useFocusSetup();
  const [notice, setNotice] = useState(''),
    [guide, setGuide] = useState(false);
  const camera = setup?.camera_permission;
  const cameraLabel =
    camera === 'granted'
      ? 'Allowed'
      : camera === 'denied'
        ? 'Blocked'
        : camera === 'restricted'
          ? 'Restricted'
          : camera === 'unsupported'
            ? 'Not supported'
            : 'Not set up';
  return (
    <section className="panel form-panel compact-settings">
      <div className="section-heading">
        <h2>Connections</h2>
        <button
          className="secondary"
          disabled={!desktop || loading}
          onClick={() => void refresh()}
        >
          Check again
        </button>
      </div>
      {(error || notice) && (
        <p role="status" className="notice">
          {error || notice}
        </p>
      )}
      <div className="setting-row">
        <div>
          <h3>Browser extension</h3>
          <p className="small muted">
            {setup?.browser.connected
              ? 'Connected'
              : 'Connect once to block websites.'}
          </p>
        </div>
        <button className="secondary" onClick={() => setGuide(!guide)}>
          {guide ? 'Hide setup' : 'Set up'}
        </button>
      </div>
      {guide && <BrowserSetup setup={setup} refresh={refresh} />}
      <div className="setting-row">
        <div>
          <h3>
            Camera <span className="badge">{cameraLabel}</span>
          </h3>
          <p className="small muted">
            Optional. Checks if you are at your desk.
          </p>
        </div>
        {camera !== 'granted' && (
          <button
            className="secondary"
            disabled={!desktop || setup?.platform !== 'macos'}
            onClick={async () => {
              try {
                await api.requestCamera();
                setNotice(
                  'Choose Allow in the macOS prompt, then click Check again.',
                );
                await refresh();
              } catch (e) {
                setNotice(String(e));
              }
            }}
          >
            Allow camera
          </button>
        )}
      </div>
      {camera !== 'granted' && setup?.platform === 'macos' && (
        <details>
          <summary>No camera prompt?</summary>
          <ol className="setup-steps">
            <li>Open System Settings → Privacy & Security → Camera.</li>
            <li>Turn on VYSTRIQ.</li>
            <li>Return here and click Check again.</li>
          </ol>
          <button
            className="secondary"
            onClick={async () => {
              try {
                await api.openSetup('camera_settings');
              } catch (e) {
                setNotice(String(e));
              }
            }}
          >
            Open camera settings
          </button>
        </details>
      )}
      <div className="setting-row">
        <div>
          <h3>App activity</h3>
          <p className="small muted">
            {setup?.native_available
              ? 'Ready. You choose when to enable it.'
              : 'Unavailable on this device.'}
          </p>
        </div>
        <span className="badge">Per session</span>
      </div>
      <details>
        <summary>Privacy & recovery</summary>
        <ul className="short-help">
          <li>
            Data stays on this device. No video, typing or page content is
            saved.
          </li>
          <li>No Accessibility or Screen Recording permission is needed.</li>
          <li>
            Emergency exit stops a session. The VY menu can quit and release
            blocks.
          </li>
          <li>Website blocking covers the connected browser profile only.</li>
          <li>App blocking hides windows. It cannot lock the device.</li>
          <li>
            Camera results are estimates. Phone detection and learning
            verification are unavailable.
          </li>
        </ul>
      </details>
    </section>
  );
}
