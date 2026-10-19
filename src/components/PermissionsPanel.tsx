import { useEffect, useState } from 'react';
import { useFocusSetup } from '../hooks/useFocusSetup';
import { api, desktop } from '../services/api';
import { settingsAction, type SettingsInfo } from '../services/settings';
import { BrowserSetup } from './BrowserSetup';
import { Modal, SettingRow } from './SettingsControls';
export function PermissionsPanel() {
  const { setup, error, refresh } = useFocusSetup();
  const [panel, setPanel] = useState<'browser' | 'camera' | null>(null);
  const [notice, setNotice] = useState('');
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<SettingsInfo>({});
  const camera = setup?.camera_permission;
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
        title="Browser"
        hint="Block websites in your connected browser."
      >
        <button
          className={
            setup?.browser.connected ? 'connected-button' : 'secondary'
          }
          disabled={!desktop}
          onClick={() => {
            setNotice('');
            setPanel('browser');
          }}
        >
          {setup?.browser.connected ? 'Connected' : 'Connect'}
        </button>
      </SettingRow>
      <SettingRow
        title="Camera"
        hint="Optional. Check whether you are at your desk."
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
          title={panel === 'browser' ? 'Browser connection' : 'Camera'}
          onClose={close}
        >
          {panel === 'browser' ? (
            setup?.browser.connected ? (
              <div className="dialog-stack">
                <p className="muted">
                  Your browser is ready for focus sessions.
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
                  After an app update, reload VYSTRIQ Focus on your browser’s
                  Extensions page.
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
                  Reconnect browser
                </button>
              </div>
            ) : (
              <BrowserSetup
                setup={setup}
                refresh={async () => {
                  await refresh();
                }}
              />
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
