import { useState } from 'react';
import { Modal } from './SettingsControls';
import { api } from '../services/api';

interface InstallHelperModalProps {
  isOpen: boolean;
  onClose: () => void;
  onCancel: () => void;
  onInstalled: () => void;
  platform?: string;
}

export function InstallHelperModal({
  isOpen,
  onClose,
  onCancel,
  onInstalled,
  platform,
}: InstallHelperModalProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const resolvedPlatform =
    platform ??
    (typeof navigator !== 'undefined' && /windows/i.test(navigator.userAgent)
      ? 'windows'
      : 'macos');
  const isWindows = resolvedPlatform === 'windows';

  if (!isOpen) return null;

  async function handleInstall() {
    setBusy(true);
    setError('');
    try {
      if (isWindows) {
        await api.setupNativeBlocking();
      } else {
        await api.installPrivilegedHelper();
      }
      const installed = await api.isPrivilegedHelperInstalled();
      if (installed) {
        onInstalled();
      } else {
        setError(
          isWindows
            ? 'Administrator permission was cancelled or not completed. Windows UAC approval is required.'
            : 'Installation was cancelled or not completed. Laptop password is required to install the helper.',
        );
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title={isWindows ? 'Enable System Blocking' : 'Install Helper'}
      onClose={onClose}
    >
      <div className="dialog-stack">
        <p>
          <strong>
            {isWindows
              ? 'Administrator Permission Required'
              : 'Privileged Blocking Helper Required'}
          </strong>
        </p>
        <p className="muted">
          {isWindows
            ? 'Preelock needs one-time Windows Administrator (UAC) permission to configure system-level website blocking across all browsers and apps.'
            : 'Preelock requires the Privileged Blocking Helper to block websites and apps across the entire system.'}
        </p>
        <p className="muted">
          {isWindows
            ? 'Your approval is confirmed only in the native Windows UAC dialog and is never seen or stored by Preelock.'
            : 'Preelock will ask for your laptop administrator password in the native macOS dialog. Your password is never seen, stored, or logged by Preelock.'}
        </p>
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
        <div
          className="row controls"
          style={{
            justifyContent: 'flex-end',
            gap: '0.75rem',
            marginTop: '0.5rem',
          }}
        >
          <button
            type="button"
            className="secondary"
            onClick={onCancel}
            disabled={busy}
          >
            Cancel Session
          </button>
          <button
            type="button"
            onClick={() => void handleInstall()}
            disabled={busy}
          >
            {busy
              ? isWindows
                ? 'Enabling…'
                : 'Installing…'
              : isWindows
                ? 'Allow Access'
                : 'Install Helper'}
          </button>
        </div>
      </div>
    </Modal>
  );
}
