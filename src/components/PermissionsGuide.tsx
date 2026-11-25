import { useEffect, useState } from 'react';
import { api, desktop } from '../services/api';
import { settingsAction, type SettingsInfo } from '../services/settings';
import { useFocusSetup } from '../hooks/useFocusSetup';
import { useAppStore } from '../state/store';
import { Modal, SettingRow, Toggle } from './SettingsControls';
import { BrowserSetup, areAllInstalledBrowsersConnected } from './BrowserSetup';

export function PermissionsGuide({
  onClose,
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
  const [helperInstalled, setHelperInstalled] = useState(false);
  const mac = setup?.platform === 'macos';
  const allBrowsersConnected = areAllInstalledBrowsersConnected(setup);
  const anyBrowserConnected = Boolean(
    setup?.browser?.connected ||
    (setup?.connected_browsers && setup.connected_browsers.length > 0),
  );
  const accessibilityGranted =
    setup?.accessibility_permission === 'granted' ||
    info.accessibility === 1 ||
    info.accessibility_permission === 'granted';

  // While the guide is open, keep checking so it detects changes
  // (helper, browser connection, accessibility, notifications, camera)
  // on its own once the user toggles them in System Settings.
  useEffect(() => {
    const timer = setInterval(() => {
      void refresh();
      if (typeof api.isPrivilegedHelperInstalled === 'function') {
        void api
          .isPrivilegedHelperInstalled()
          .then((v) => setHelperInstalled(v))
          .catch(() => {});
      }
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
    if (typeof api.isPrivilegedHelperInstalled === 'function') {
      try {
        setHelperInstalled(await api.isPrivilegedHelperInstalled());
      } catch {
        // ignore
      }
    }
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
      if (typeof api.isPrivilegedHelperInstalled === 'function') {
        void api
          .isPrivilegedHelperInstalled()
          .then((v) => {
            if (active) setHelperInstalled(v);
          })
          .catch(() => {});
      }
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
  async function handleInstallHelper() {
    return act(async () => {
      if (mac && typeof api.installPrivilegedHelper === 'function') {
        await api.installPrivilegedHelper();
      } else if (typeof api.setupNativeBlocking === 'function') {
        await api.setupNativeBlocking();
      }
      if (typeof api.isPrivilegedHelperInstalled === 'function') {
        const installed = await api.isPrivilegedHelperInstalled();
        setHelperInstalled(installed);
      }
    });
  }

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
              title="Privileged Blocking Helper"
              hint={
                helperInstalled
                  ? 'Helper is installed and enforcing website blocking across all apps.'
                  : 'Required to block websites and apps across your system without browser extensions.'
              }
            >
              <button
                className={helperInstalled ? 'secondary' : undefined}
                disabled={busy || helperInstalled}
                onClick={() => void handleInstallHelper()}
              >
                {helperInstalled ? 'Installed' : 'Install Helper'}
              </button>
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
                allBrowsersConnected
                  ? 'All installed browsers are connected to Preelock.'
                  : anyBrowserConnected
                    ? `Connected: ${setup?.connected_browsers?.length ?? 1} browser(s). You can connect additional browsers anytime.`
                    : 'Connect your browser so Preelock can block distracting sites during sessions.'
              }
            >
              <button
                className="secondary"
                disabled={allBrowsersConnected}
                onClick={() => setBrowserGuide(true)}
              >
                {allBrowsersConnected ? 'Connected' : 'Connect browser'}
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
          <button
            onClick={onClose}
            disabled={busy || (!helperInstalled && desktop)}
          >
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
  const [helperInstalled, setHelperInstalled] = useState(false);
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

  useEffect(() => {
    if (!desktop) return;
    let active = true;
    if (typeof api.isPrivilegedHelperInstalled === 'function') {
      api
        .isPrivilegedHelperInstalled()
        .then((v) => {
          if (active) setHelperInstalled(v);
        })
        .catch(() => {});
    }
    return () => {
      active = false;
    };
  }, []);

  // Never show onboarding if backend database confirms it is already completed
  const onboardingDone = Boolean(data?.preferences?.onboarding_completed);

  // Never interrupt an active running or preparing session (Requirement 1 & 6)
  const hasActiveSession = data?.sessions?.some(
    (s) =>
      s.status === 'running' ||
      s.status === 'preparing' ||
      s.status === 'paused',
  );

  // Mandatory: On fresh installation, helper must be installed. Browser connection is optional.
  const mac = setup?.platform === 'macos';
  const accessibilityOk = !mac || setup?.accessibility_permission === 'granted';
  const helperOk = !desktop || helperInstalled;
  const alreadyGrantedOnFirstRun = Boolean(
    open && setup && accessibilityOk && helperOk,
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
