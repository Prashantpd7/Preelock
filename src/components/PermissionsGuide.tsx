import { useEffect, useState } from 'react';
import { api, desktop } from '../services/api';
import { settingsAction, type SettingsInfo } from '../services/settings';
import { useFocusSetup } from '../hooks/useFocusSetup';
import { Modal, SettingRow, Toggle } from './SettingsControls';
import { BrowserSetup } from './BrowserSetup';

export function PermissionsGuide({ onClose }: { onClose: () => void }) {
  const { setup, refresh } = useFocusSetup();
  const [info, setInfo] = useState<SettingsInfo>({});
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [busy, setBusy] = useState(false);
  const [browserGuide, setBrowserGuide] = useState(false);
  const mac = setup?.platform === 'macos';
  const browserConnected = !!setup?.browser.connected && !setup?.browser.error;
  // While the guide is open, keep checking so it detects the connection
  // on its own once the user connects in the browser.
  useEffect(() => {
    if (!browserGuide || browserConnected) return;
    const timer = setInterval(() => {
      void refresh();
    }, 3000);
    return () => clearInterval(timer);
  }, [browserGuide, browserConnected, refresh]);
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
    setNotice('');
    try {
      await action();
      await check();
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    } finally {
      setBusy(false);
    }
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
                aria-label={cameraDenied ? 'Open settings' : undefined}
                onClick={() =>
                  void (async () => {
                    const ok = await act(() =>
                      cameraDenied
                        ? api.openSetup('camera_settings')
                        : api.requestCamera(),
                    );
                    // Permission only: requestCamera asks macOS for
                    // authorization and never starts the camera or a
                    // session. Tell the user what to do next instead of
                    // leaving the row unchanged while the system
                    // prompt is up (or System Settings is open).
                    if (ok) {
                      setNotice(
                        cameraDenied
                          ? 'Turn on Preelock under Camera, then return here.'
                          : 'Choose Allow in the Mac permission window.',
                      );
                    }
                  })()
                }
              >
                {!mac
                  ? 'Unavailable'
                  : setup.camera_permission === 'granted'
                    ? 'Allowed'
                    : cameraDenied
                      ? 'Denied — System Settings'
                      : 'Allow camera'}
              </button>
            </SettingRow>
            {!mac && (
              <p className="muted small">
                If Windows SmartScreen appears, choose{' '}
                <strong>More info → Run anyway</strong> to continue.
              </p>
            )}
            <SettingRow
              title="Open at login"
              hint={
                info.login_user_pref && info.login === 2
                  ? 'Needs approval in System Settings to open at login.'
                  : 'Opens the app at login.'
              }
            >
              <Toggle
                label="Open at login"
                checked={info.login_user_pref ?? false}
                disabled={busy || info.login_user_pref === undefined}
                onChange={(enabled) =>
                  void act(() => settingsAction({ type: 'login', enabled }))
                }
              />
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
        {notice && (
          <p role="status" className="notice">
            {notice}
          </p>
        )}
        <div className="dialog-footer">
          <button onClick={onClose}>Done</button>
        </div>
      </Modal>
      {browserGuide && !browserConnected && (
        <Modal
          title="Browser connection"
          onClose={() => setBrowserGuide(false)}
        >
          <div className="dialog-stack">
            <BrowserSetup setup={setup} refresh={refresh} allowAdd={false} />
          </div>
        </Modal>
      )}
    </>
  );
}

export function FirstRunPermissions() {
  const [open, setOpen] = useState(() => {
    try {
      return (
        desktop &&
        localStorage.getItem('preelock-permissions-v2') !== 'seen' &&
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
          localStorage.setItem('preelock-permissions-v2', 'seen');
          localStorage.setItem('preelock-permissions-v10', 'seen');
        } catch {
          /* Still dismiss for this run. */
        }
        setOpen(false);
      }}
    />
  );
}
