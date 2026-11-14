import { useState } from 'react';
import { AlertTriangle } from 'lucide-react';
import { useAppStore } from '../state/store';
import { useFocusSetup } from '../hooks/useFocusSetup';
import { BrowserSetup } from './BrowserSetup';
import { Modal } from './SettingsControls';

/**
 * Global persistent red warning banner rendered at the top of every page
 * whenever the user actively uses an unconnected supported browser.
 */
export function UnconnectedBrowserBanner() {
  const warning = useAppStore((s) => s.data.unconnected_browser);
  const [connectModalOpen, setConnectModalOpen] = useState(false);
  const { setup, refresh } = useFocusSetup(warning?.browser_id);

  if (!warning) return null;

  return (
    <>
      <div
        className="unconnected-browser-banner"
        role="alert"
        aria-live="polite"
      >
        <div className="banner-message">
          <AlertTriangle size={18} className="banner-icon" />
          <span>
            {warning.browser_name} is not connected to Preelock. Connect it to
            enable browser tracking and protection.
          </span>
        </div>
        <button
          type="button"
          className="banner-connect-button"
          onClick={() => setConnectModalOpen(true)}
        >
          Connect
        </button>
      </div>

      {connectModalOpen && (
        <Modal
          title={`Connect ${warning.browser_name}`}
          onClose={() => setConnectModalOpen(false)}
        >
          <BrowserSetup
            browserId={warning.browser_id}
            setup={setup}
            refresh={refresh}
            onBack={() => setConnectModalOpen(false)}
          />
        </Modal>
      )}
    </>
  );
}
