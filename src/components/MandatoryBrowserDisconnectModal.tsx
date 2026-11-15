import { useFocusSetup } from '../hooks/useFocusSetup';
import { useAppStore } from '../state/store';
import { desktop } from '../services/api';
import {
  areAllInstalledBrowsersConnected,
  getUnconnectedInstalledBrowsers,
  BrowserSetup,
} from './BrowserSetup';
import { Modal } from './SettingsControls';

/**
 * Mandatory persistent modal that opens when any installed browser (except Safari on macOS)
 * gets disconnected in the future. Cannot be dismissed until all installed browsers are connected.
 */
export function MandatoryBrowserDisconnectModal() {
  const { setup, refresh } = useFocusSetup();
  const isPaused = useAppStore((s) => Boolean(s.data?.preferences?.paused));

  const firstRunCompleted = (() => {
    try {
      return (
        Boolean(
          useAppStore.getState().data?.preferences?.onboarding_completed,
        ) ||
        localStorage.getItem('preelock-permissions-v10') === 'seen' ||
        localStorage.getItem('preelock-permissions-v2') === 'seen'
      );
    } catch {
      return false;
    }
  })();

  const hadAllConnected = (() => {
    try {
      return localStorage.getItem('preelock-all-browsers-connected') === 'true';
    } catch {
      return false;
    }
  })();

  const allConnected = areAllInstalledBrowsersConnected(setup);

  if (allConnected) {
    try {
      localStorage.setItem('preelock-all-browsers-connected', 'true');
    } catch {
      // ignore
    }
    return null;
  }

  if (
    !desktop ||
    !firstRunCompleted ||
    !hadAllConnected ||
    isPaused ||
    !setup
  ) {
    return null;
  }

  const unconnected = getUnconnectedInstalledBrowsers(setup);

  return (
    <Modal title="Browser connection required" hideClose={true}>
      <div className="dialog-stack">
        <p
          className="connect-warn"
          role="alert"
          style={{ marginBottom: '1rem' }}
        >
          {unconnected.length === 1
            ? `${unconnected[0].name} is not connected.`
            : `The following browsers are not connected: ${unconnected.map((b) => b.name).join(', ')}.`}{' '}
          Connect all installed browsers to continue using Preelock.
        </p>
        <BrowserSetup setup={setup} refresh={refresh} allowAdd={false} />
      </div>
    </Modal>
  );
}
