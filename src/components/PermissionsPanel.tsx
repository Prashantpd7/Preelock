import { useEffect, useState } from 'react';
import { RefreshCw } from 'lucide-react';
import { useFocusSetup } from '../hooks/useFocusSetup';
import { api, desktop } from '../services/api';
import { settingsAction, type SettingsInfo } from '../services/settings';
import { hasLiveStrictSession } from '../services/activitySection';
import { useAppStore } from '../state/store';
import { PermissionsGuide } from './PermissionsGuide';
import { BrowserSetup } from './BrowserSetup';
import { Modal, SettingRow } from './SettingsControls';
export function PermissionsPanel() {
  const { setup, error, refresh } = useFocusSetup();
  const [panel, setPanel] = useState<
    'browser' | 'add-browser' | 'camera' | null
  >(null);
  const [notice, setNotice] = useState('');
  const [busy, setBusy] = useState(false);
  const [lockedDialog, setLockedDialog] = useState(false);
  const [status, setStatus] = useState<SettingsInfo>({});
  // Browser connection management is locked while a strict focus
  // session is live; the backend enforces the same lock, so this is
  // only the visible half (see `reset_pairing` and the bridge
  // connection lock in the Rust backend).
  const strictLive = hasLiveStrictSession(useAppStore((s) => s.data));
  const [permissions, setPermissions] = useState(false);
  const camera = setup?.camera_permission;
  // Several browsers can stay paired at once; the row reflects all of them.
  const connectedIds = setup?.connected_browsers ?? [];
  const getBrowserName = (id: string) => {
    const found = setup?.browsers?.find(
      (b) => b.id.toLowerCase() === id.toLowerCase(),
    );
    if (found?.name) return found.name;
    const lower = id.toLowerCase();
    if (lower.includes('comet')) return 'Comet';
    if (lower.includes('chrome')) return 'Google Chrome';
    if (lower.includes('brave')) return 'Brave';
    if (lower.includes('edge') || lower.includes('edgemac'))
      return 'Microsoft Edge';
    if (lower.includes('vivaldi')) return 'Vivaldi';
    if (lower.includes('chromium')) return 'Chromium';
    return id;
  };
  const connectedNames = connectedIds.map(getBrowserName);
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
  const [refreshing, setRefreshing] = useState(false);
  async function handleReconnectBrowser() {
    setRefreshing(true);
    setNotice('');
    try {
      await settingsAction({ type: 'reconnect_browser' }).catch(() => {});
      await refresh();
      await useAppStore.getState().refresh();
    } catch (e) {
      setNotice(String(e));
    } finally {
      window.setTimeout(() => setRefreshing(false), 400);
    }
  }
  async function allowCamera() {
    setBusy(true);
    setNotice('');
    try {
      if (camera === 'denied' || camera === 'restricted') {
        await api.openSetup('camera_settings');
        setNotice('Turn on Preelock under Camera, then return here.');
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
                <div
                  style={{
                    display: 'flex',
                    flexDirection: 'column',
                    gap: '0.625rem',
                  }}
                >
                  {connectedIds.map((id) => {
                    const name = getBrowserName(id);
                    return (
                      <div
                        key={id}
                        style={{
                          display: 'flex',
                          alignItems: 'center',
                          justifyContent: 'space-between',
                          padding: '0.875rem 1rem',
                          borderRadius: '8px',
                          backgroundColor: 'rgba(255, 255, 255, 0.04)',
                          border: '1px solid rgba(255, 255, 255, 0.08)',
                        }}
                      >
                        <div
                          style={{
                            display: 'flex',
                            alignItems: 'center',
                            gap: '0.75rem',
                          }}
                        >
                          <span
                            style={{
                              width: '8px',
                              height: '8px',
                              borderRadius: '50%',
                              backgroundColor: '#22c55e',
                              boxShadow: '0 0 6px rgba(34, 197, 94, 0.6)',
                              display: 'inline-block',
                            }}
                            aria-hidden="true"
                          />
                          <div>
                            <div
                              style={{
                                fontWeight: 500,
                                fontSize: '0.95rem',
                                color: '#f8fafc',
                              }}
                            >
                              Connected in {name}
                            </div>
                            <div
                              style={{
                                fontSize: '0.8rem',
                                color: 'rgba(255, 255, 255, 0.5)',
                                marginTop: '0.125rem',
                              }}
                            >
                              Extension is active and enforcing rules
                            </div>
                          </div>
                        </div>
                        <button
                          type="button"
                          className="icon-button"
                          title="Refresh connection"
                          aria-label="Refresh connection"
                          disabled={refreshing || busy}
                          onClick={() => void handleReconnectBrowser()}
                          style={{
                            background: 'rgba(255, 255, 255, 0.06)',
                            border: '1px solid rgba(255, 255, 255, 0.1)',
                            cursor: 'pointer',
                            padding: '0.5rem',
                            borderRadius: '6px',
                            display: 'flex',
                            alignItems: 'center',
                            justifyContent: 'center',
                            color: '#f8fafc',
                          }}
                        >
                          <RefreshCw
                            size={15}
                            className={refreshing ? 'spin' : undefined}
                          />
                        </button>
                      </div>
                    );
                  })}
                </div>
                <button
                  className="secondary"
                  onClick={() => {
                    if (strictLive) {
                      setPanel(null);
                      setLockedDialog(true);
                      return;
                    }
                    setPanel('add-browser');
                  }}
                >
                  Connect another browser
                </button>
                {strictLive && (
                  <p className="small muted">
                    Browser settings are locked during an active focus session.
                  </p>
                )}
                <div>
                  <button
                    className="text-button"
                    disabled={busy || refreshing}
                    onClick={async () => {
                      if (strictLive) {
                        setPanel(null);
                        setLockedDialog(true);
                        return;
                      }
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
                    Disconnect every browser
                  </button>
                </div>
              </div>
            ) : (
              <div className="dialog-stack">
                <BrowserSetup
                  setup={setup}
                  refresh={async () => {
                    await refresh();
                  }}
                  allowAdd={panel === 'add-browser'}
                  onBack={
                    anyBrowserConnected ? () => setPanel('browser') : undefined
                  }
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
    </section>
  );
}
