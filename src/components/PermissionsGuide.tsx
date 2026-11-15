import { useEffect, useState } from 'react';
import { api, desktop } from '../services/api';
import { settingsAction, type SettingsInfo } from '../services/settings';
import { useFocusSetup } from '../hooks/useFocusSetup';
import { useAppStore } from '../state/store';
import { Modal, SettingRow, Toggle } from './SettingsControls';
import {
  BrowserSetup,
  areAllInstalledBrowsersConnected,
  getInstalledSupportedBrowsers,
  getUnconnectedInstalledBrowsers,
} from './BrowserSetup';

export function PermissionsGuide({
  onClose,
  isFirstRun = false,
}: {
  onClose: () => void;
  isFirstRun?: boolean;
}) {
  const { setup, refresh } = useFocusSetup();
  const [info, setInfo] = useState<SettingsInfo>({});
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [busy, setBusy] = useState(false);
  const [browserGuide, setBrowserGuide] = useState(false);
  const mac = setup?.platform === 'macos';
  const installedBrowsers = getInstalledSupportedBrowsers(setup);
  const unconnectedBrowsers = getUnconnectedInstalledBrowsers(setup);
  const allBrowsersConnected = areAllInstalledBrowsersConnected(setup);
  const browserConnected = allBrowsersConnected;
  const accessibilityGranted =
    setup?.accessibility_permission === 'granted' ||
    info.accessibility === 1 ||
    info.accessibility_permission === 'granted';

  // While the guide is open, keep checking so it detects changes
  // (browser connection, accessibility, notifications, camera)
  // on its own once the user toggles them in System Settings.
  useEffect(() => {
    const timer = setInterval(() => {
      void refresh();
      void settingsAction({ type: 'recheck_permissions' })
        .then((v) => setInfo(v))
        .catch(() => {
          void settingsAction({ type: 'info' })
            .then((v) => setInfo(v))
            .catch(() => {});
        });
    }, 2000);
    return () => clearInterval(timer);
  }, [refresh]);
  async function check() {
    await refresh();
    try {
      setInfo(await settingsAction({ type: 'recheck_permissions' }));
    } catch {
      try {
        setInfo(await settingsAction({ type: 'info' }));
      } catch (e) {
        setError(String(e));
      }
    }
  }
  useEffect(() => {
    let active = true;
    const read = () => {
      void refresh();
      void settingsAction({ type: 'recheck_permissions' })
        .then((v) => {
          if (active) setInfo(v);
        })
        .catch(() => {
          void settingsAction({ type: 'info' })
            .then((v) => {
              if (active) setInfo(v);
            })
            .catch((e) => {
              if (active) setError(String(e));
            });
        });
    };
    read();
    window.addEventListener('focus', read);
    return () => {
      active = false;
      window.removeEventListener('focus', read);
    };
  }, [refresh]);
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
      <Modal
        title="Permissions & setup"
        onClose={onClose}
        hideClose={isFirstRun && !browserConnected}
      >
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
                info.notifications === 1
                  ? mac
                    ? 'Notifications are turned off in System Settings → Notifications → Preelock.'
                    : 'Notifications are turned off in Settings → System → Notifications → Preelock.'
                  : mac
                    ? 'System Settings → Notifications → Preelock → Allow notifications.'
                    : 'Settings → System → Notifications → turn on Preelock. Install the app first; Do not disturb may silence alerts.'
              }
            >
              <button
                className="secondary"
                disabled={busy || info.notifications === 2}
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
                  : info.notifications === 2
                    ? 'Allowed'
                    : mac && info.notifications === 0
                      ? 'Allow notifications'
                      : 'Open settings'}
              </button>
            </SettingRow>
            {mac && (
              <SettingRow
                title="Accessibility"
                hint={
                  accessibilityGranted
                    ? 'Screen Time activity tracking and distraction blocking are active.'
                    : 'Required for Screen Time activity tracking and distraction blocking. Toggle on in System Settings.'
                }
              >
                <button
                  className="secondary"
                  disabled={busy || accessibilityGranted}
                  onClick={() =>
                    void act(() => api.openSetup('accessibility_settings'))
                  }
                >
                  {accessibilityGranted ? 'Allowed' : 'Open settings'}
                </button>
              </SettingRow>
            )}
            <SettingRow
              title="Browser extension"
              hint={
                browserConnected
                  ? 'All installed browsers are connected to Preelock.'
                  : installedBrowsers.length > 0
                    ? `Connect all installed browsers: ${unconnectedBrowsers.map((b) => b.name).join(', ')} (${installedBrowsers.length - unconnectedBrowsers.length}/${installedBrowsers.length} connected). Required to continue.`
                    : 'Connect your browser so Preelock can block distracting sites during sessions.'
              }
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
          <button disabled={!browserConnected} onClick={onClose}>
            Done
          </button>
        </div>
      </Modal>
      {browserGuide && (
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
  const data = useAppStore((s) => s.data);
  const { setup } = useFocusSetup();
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

  // Never show onboarding if backend database confirms it is already completed
  const onboardingDone = Boolean(data?.preferences?.onboarding_completed);

  // Never interrupt an active running or preparing session (Requirement 1 & 6)
  const hasActiveSession = data?.sessions?.some(
    (s) =>
      s.status === 'running' ||
      s.status === 'preparing' ||
      s.status === 'paused',
  );

  // Requirement 5.3: On fresh installation, if capabilities are already granted, continue without displaying prompt
  const mac = setup?.platform === 'macos';
  const accessibilityOk = !mac || setup?.accessibility_permission === 'granted';
  const allBrowsersOk = areAllInstalledBrowsersConnected(setup);
  const alreadyGrantedOnFirstRun = Boolean(
    open && setup && accessibilityOk && allBrowsersOk,
  );

  useEffect(() => {
    if (alreadyGrantedOnFirstRun) {
      try {
        localStorage.setItem('preelock-permissions-v10', 'seen');
        localStorage.setItem('preelock-all-browsers-connected', 'true');
      } catch {
        // ignore
      }
      if (data?.preferences && !data.preferences.onboarding_completed) {
        void api
          .execute({
            type: 'update_preferences',
            preferences: { ...data.preferences, onboarding_completed: true },
          })
          .catch(() => {});
      }
    }
  }, [alreadyGrantedOnFirstRun, data?.preferences]);

  if (!open || onboardingDone || hasActiveSession || alreadyGrantedOnFirstRun) {
    return null;
  }

  return (
    <PermissionsGuide
      isFirstRun={true}
      onClose={() => {
        try {
          localStorage.setItem('preelock-permissions-v2', 'seen');
          localStorage.setItem('preelock-permissions-v10', 'seen');
          localStorage.setItem('preelock-all-browsers-connected', 'true');
        } catch {
          /* Still dismiss for this run. */
        }
        if (data?.preferences) {
          void api
            .execute({
              type: 'update_preferences',
              preferences: { ...data.preferences, onboarding_completed: true },
            })
            .catch(() => {});
        }
        setOpen(false);
      }}
    />
  );
}
