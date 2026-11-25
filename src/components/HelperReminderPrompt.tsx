import { useEffect, useState, useRef } from 'react';
import { api, desktop } from '../services/api';
import { Modal } from './SettingsControls';

export function HelperReminderPrompt({
  initialDelayMs,
}: {
  initialDelayMs?: number;
} = {}) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!desktop) return;

    let active = true;

    async function checkHelper() {
      if (!active) return;
      try {
        const installed = await api.isPrivilegedHelperInstalled();
        if (!installed && active) {
          setOpen(true);
        } else if (active) {
          scheduleNextCheck();
        }
      } catch {
        if (active) scheduleNextCheck();
      }
    }

    function scheduleNextCheck() {
      if (timerRef.current) clearTimeout(timerRef.current);
      // Random delay between 45 and 90 seconds
      const delay = Math.floor(45000 + Math.random() * 45000);
      timerRef.current = setTimeout(() => {
        void checkHelper();
      }, delay);
    }

    // Initial check
    const initialDelay =
      initialDelayMs ?? Math.floor(10000 + Math.random() * 10000);
    timerRef.current = setTimeout(() => {
      void checkHelper();
    }, initialDelay);

    return () => {
      active = false;
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, [initialDelayMs]);

  if (!open) return null;

  async function handleInstall() {
    setBusy(true);
    setError('');
    try {
      await api.installPrivilegedHelper();
      const installed = await api.isPrivilegedHelperInstalled();
      if (installed) {
        setOpen(false);
      } else {
        setError(
          'Installation was not completed. Laptop password is required.',
        );
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  function handleDismiss() {
    setOpen(false);
    // Schedule next prompt after random interval (60-120 seconds)
    if (timerRef.current) clearTimeout(timerRef.current);
    const delay = Math.floor(60000 + Math.random() * 60000);
    timerRef.current = setTimeout(async () => {
      try {
        const installed = await api.isPrivilegedHelperInstalled();
        if (!installed) {
          setOpen(true);
        }
      } catch {
        // ignore
      }
    }, delay);
  }

  return (
    <Modal title="Install Helper" onClose={handleDismiss}>
      <div className="dialog-stack" style={{ maxWidth: '380px' }}>
        <p
          style={{
            margin: '0.25rem 0 0.75rem 0',
            fontSize: '14px',
            lineHeight: 1.5,
          }}
        >
          Install helper to detect and block websites across all apps.
        </p>
        {error && (
          <p
            role="alert"
            className="error"
            style={{ marginBottom: '0.75rem', fontSize: '12px' }}
          >
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
            onClick={handleDismiss}
            disabled={busy}
          >
            Cancel
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
