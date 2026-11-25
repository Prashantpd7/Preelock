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
  const isWindows = platform === 'windows';

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
          'Installation was cancelled or not completed. Laptop password is required to install the helper.',
        );
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title="Install Helper" onClose={onClose}>
      <div className="dialog-stack">
        <p>
          <strong>Privileged Blocking Helper Required</strong>
        </p>
        <p className="muted" style={{ margin: '0.75rem 0', lineHeight: 1.5 }}>
          Preelock requires the Privileged Blocking Helper to block websites and
          apps across the entire system.
        </p>
        <p className="muted" style={{ margin: '0 0 1rem 0', lineHeight: 1.5 }}>
          {isWindows
            ? 'This requires a one-time administrator approval to configure system-level website blocking. Your credentials are confirmed only in the native Windows dialog.'
            : 'Preelock will ask for your laptop administrator password in the native macOS dialog. Your password is never seen, stored, or logged by Preelock.'}
        </p>
        {error && (
          <p role="alert" className="error" style={{ marginBottom: '1rem' }}>
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
            {busy ? 'Installing…' : 'Install Helper'}
          </button>
        </div>
      </div>
    </Modal>
  );
}
