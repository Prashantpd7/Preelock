import { useEffect, useState } from 'react';
import { useFocusSetup } from '../hooks/useFocusSetup';
import { api, desktop } from '../services/api';
import { settingsAction, type SettingsInfo } from '../services/settings';
import { PermissionsGuide } from './PermissionsGuide';
import { BrowserSetup } from './BrowserSetup';
import { Modal, SettingRow } from './SettingsControls';
export function PermissionsPanel() {
  const [pairBrowser, setPairBrowser] = useState('');
  const { setup, error, refresh } = useFocusSetup(pairBrowser || undefined);
  const [panel, setPanel] = useState<
    'browser' | 'add-browser' | 'camera' | null
  >(null);
  const [notice, setNotice] = useState('');
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<SettingsInfo>({});
  const [permissions, setPermissions] = useState(false);
  const camera = setup?.camera_permission;
  // Several browsers can stay paired at once; the row reflects all of them.
  const connectedIds = setup?.connected_browsers ?? [];
  const connectedNames = connectedIds.map(
    (id) => setup?.browsers?.find((b) => b.id === id)?.name ?? id,
  );
  const anyBrowserConnected = connectedNames.length > 0;
  useEffect(() => {
    if (!desktop || !panel) return;
    const timer = window.setInterval(() => {
      void refresh();
      void settingsAction({ type: 'status' })
        .then(setStatus)
        .catch((e) => setNotice(String(e)));
    }, 2000);
    return () => window.clearInterval(timer);
  }, [panel, refresh]);
  async function allowCamera() {
    setBusy(true);
    setNotice('');
    try {
      if (camera === 'denied' || camera === 'restricted') {
        await api.openSetup('camera_settings');
        setNotice('Turn on VYSTRIQ under Camera, then return here.');
      } else {
        await api.requestCamera();
        setNotice('Choose Allow in the Mac permission window.');
      }
      setPanel('camera');
      await refresh();
    } catch (e) {
      setNotice(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function test(
    type: 'test_camera' | 'stop_camera_test' | 'test_browser',
  ) {
    setBusy(true);
    setNotice('');
    try {
      await settingsAction({ type });
      setStatus(await settingsAction({ type: 'status' }));
    } catch (e) {
      setNotice(String(e));
    } finally {
      setBusy(false);
    }
  }
  function close() {
    if (panel === 'camera')
      void settingsAction({ type: 'stop_camera_test' }).catch(() => {});
    setPanel(null);
    setNotice('');
  }
  const cameraRunning =
    !!status.camera_status &&
    !['off', 'unsupported'].includes(status.camera_status);
  const cameraText: Record<string, string> = {
    person_detected: 'You are in view',
    person_not_detected: 'Move into view',
    extended_absence: 'Move into view',
    starting: 'Starting camera…',
    off: 'Camera is off',
    camera_unavailable: 'Camera is in use or unavailable',
    camera_stalled: 'Camera stopped responding',
    analysis_error: 'Could not check the image',
  };
  return (
    <section className="settings-section">
      <h2>Connections</h2>
      <SettingRow
        title="Permissions"
        hint="Camera, notifications and setup help."
      >
        <button className="secondary" onClick={() => setPermissions(true)}>
          Open guide
        </button>
      </SettingRow>
      {permissions && (
        <PermissionsGuide
          onClose={() => {
            setPermissions(false);
            void refresh();
          }}
        />
      )}
      <SettingRow
        title="Browser"
        hint={
          anyBrowserConnected
            ? `Connected in: ${connectedNames.join(', ')}.`
            : 'Block websites in your connected browser.'
        }
      >
        <button
          className={anyBrowserConnected ? 'connected-button' : 'secondary'}
          disabled={!desktop}
          onClick={() => {
            setNotice('');
            setPanel('browser');
          }}
        >
          {anyBrowserConnected ? 'Connected' : 'Connect'}
        </button>
      </SettingRow>
      <SettingRow
        title="Camera"
        hint={
          setup?.platform === 'windows'
            ? 'Not available on Windows. Use Apps only.'
            : 'Optional. Check whether you are at your desk.'
        }
      >
        <button
          className={camera === 'granted' ? 'connected-button' : 'secondary'}
          disabled={!desktop || busy || setup?.platform !== 'macos'}
          onClick={() =>
            camera === 'granted' ? setPanel('camera') : void allowCamera()
          }
        >
          {camera === 'granted'
            ? 'Allowed'
            : camera === 'unsupported'
              ? 'Unavailable'
              : 'Allow camera'}
        </button>
      </SettingRow>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {panel && (
        <Modal
          title={panel === 'camera' ? 'Camera' : 'Browser connection'}
          onClose={close}
        >
          {panel !== 'camera' ? (
            anyBrowserConnected && panel !== 'add-browser' ? (
              <div className="dialog-stack">
                <p className="muted">
                  Connected in: {connectedNames.join(', ')}.
                </p>
                <button
                  className="secondary"
                  disabled={busy}
                  onClick={() => void test('test_browser')}
                >
                  Test blocking
                </button>
                {status.browser_test && (
                  <p role="status">{status.browser_test}</p>
                )}
                <p className="small muted">
                  After an app update, reload Vystriq on each browser's
                  Extensions page.
                </p>
                <button
                  className="secondary"
                  onClick={() => setPanel('add-browser')}
                >
                  Connect another browser
                </button>
                <p className="small muted">
                  Resetting disconnects every browser; each one will need the
                  connection code again.
                </p>
                <button
                  className="text-button"
                  disabled={busy}
                  onClick={async () => {
                    setBusy(true);
                    try {
                      await api.resetPairing();
                      await refresh();
                    } catch (e) {
                      setNotice(String(e));
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  Reset all browser connections
                </button>
              </div>
            ) : (
              <div className="dialog-stack">
                <div className="choice-group" aria-label="Browser to connect">
                  {setup?.browsers?.map((browser) => (
                    <button
                      key={browser.id}
                      className="secondary"
                      aria-pressed={pairBrowser === browser.id}
                      onClick={() => setPairBrowser(browser.id)}
                    >
                      {browser.name}
                    </button>
                  ))}
                </div>
                <BrowserSetup
                  browserId={pairBrowser}
                  setup={setup}
                  refresh={async () => {
                    await refresh();
                  }}
                />
              </div>
            )
          ) : camera === 'granted' ? (
            <div className="dialog-stack">
              <p className="muted">
                Test for up to 30 seconds. No video is saved.
              </p>
              <button
                className="secondary"
                disabled={busy}
                onClick={() =>
                  void test(cameraRunning ? 'stop_camera_test' : 'test_camera')
                }
              >
                {cameraRunning ? 'Stop test' : 'Test camera'}
              </button>
              <p role="status">
                {cameraText[status.camera_status ?? 'off'] ??
                  'Camera unavailable. Try again.'}
              </p>
            </div>
          ) : (
            <p>
              {notice ||
                'Approve camera access in System Settings, then return here.'}
            </p>
          )}
          {notice && !(panel === 'camera' && camera !== 'granted') && (
            <p role="status" className="notice">
              {camera === 'granted' && notice.startsWith('Choose Allow')
                ? 'Camera access is ready.'
                : notice}
            </p>
          )}
        </Modal>
      )}
    </section>
  );
}
