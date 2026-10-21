import { useEffect, useState } from 'react';
import { api, desktop } from '../services/api';
import { settingsAction, type SettingsInfo } from '../services/settings';
import { useFocusSetup } from '../hooks/useFocusSetup';
import { Modal, SettingRow, Toggle } from './SettingsControls';

export function PermissionsGuide({ onClose }: { onClose: () => void }) {
  const { setup, refresh } = useFocusSetup();
  const [info, setInfo] = useState<SettingsInfo>({});
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const mac = setup?.platform === 'macos';
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
    try {
      await action();
      await check();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  const cameraDenied = ['denied', 'restricted'].includes(
    setup?.camera_permission ?? '',
  );
  return (
    <Modal title="Permissions & setup" onClose={onClose}>
      {!setup ? (
        <p>Checking your system…</p>
      ) : (
        <div className="dialog-stack permission-guide">
          <SettingRow
            title="App control"
            hint={
              mac
                ? 'Hides distracting apps during sessions. Accessibility and Screen Recording access are not required.'
                : 'Minimizes distracting windows during sessions. Administrator access is not required.'
            }
          >
            <span>Ready</span>
          </SettingRow>
          <SettingRow
            title="Camera"
            hint={
              !mac
                ? 'Not available on Windows. Use Apps only.'
                : cameraDenied
                  ? 'Denied — System Settings → Privacy & Security → Camera → turn on Preelock.'
                  : 'Optional. Permission is remembered by the system. Camera runs only in sessions where you select it (or during a camera test). No video is saved.'
            }
          >
            <button
              className="secondary"
              disabled={busy || !mac || setup.camera_permission === 'granted'}
              onClick={() =>
                void act(() =>
                  cameraDenied
                    ? api.openSetup('camera_settings')
                    : api.requestCamera(),
                )
              }
            >
              {!mac
                ? 'Unavailable'
                : setup.camera_permission === 'granted'
                  ? 'Allowed'
                  : cameraDenied
                    ? 'Open settings'
                    : 'Allow camera'}
            </button>
          </SettingRow>
          <SettingRow
            title="Open at login"
            hint={
              info.login === 2
                ? 'Approve Preelock in System Settings → General → Login Items.'
                : 'Allow automatic reopening after you sign in. Unfinished sessions recover after shutdown or restart.'
            }
          >
            <Toggle
              label="Open at login"
              checked={info.login === 1 || info.login === 2}
              disabled={busy || info.login === undefined || info.login === -1}
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
            hint="Connect each browser you use. Enable Allow in Incognito on its Extensions page to block private windows too."
          >
            <span>Use Connections</span>
          </SettingRow>
          <section className="setup-guide">
            <h3>Opening a downloaded app</h3>
            <p>Continue only if you trust where you downloaded Preelock.</p>
            {mac ? (
              <ol>
                <li>
                  Drag Preelock from the downloaded disk image into
                  Applications, then open the installed app from Finder.
                </li>
                <li>
                  If blocked, open System Settings → Privacy & Security → Open
                  Anyway, if offered, then confirm Open. This free build is not
                  Apple-notarized.
                </li>
                <li>
                  If approval is unavailable or the app is reported as damaged,
                  check the release installation guide. Do not disable system
                  protection.
                </li>
              </ol>
            ) : (
              <ol>
                <li>
                  If SmartScreen says “Windows protected your PC,” choose More
                  info → Run anyway, if offered.
                </li>
                <li>
                  If your organization or Smart App Control blocks it, ask your
                  administrator. Do not turn off system protection.
                </li>
              </ol>
            )}
            {mac && (
              <button
                className="secondary"
                onClick={() =>
                  void act(() => api.openSetup('privacy_settings'))
                }
              >
                Open Privacy & Security
              </button>
            )}
          </section>
        </div>
      )}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <div className="dialog-footer">
        <button onClick={onClose}>Done</button>
      </div>
    </Modal>
  );
}

export function FirstRunPermissions() {
  const [open, setOpen] = useState(() => {
    try {
      return (
        desktop &&
        localStorage.getItem('preelock-permissions-v2') !== 'seen' &&
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
        } catch {
          /* Still dismiss for this run. */
        }
        setOpen(false);
      }}
    />
  );
}
