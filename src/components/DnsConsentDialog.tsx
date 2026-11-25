import { Modal } from './SettingsControls';

interface DnsConsentDialogProps {
  onAllow: () => void;
  onDecline: () => void;
  onClose?: () => void;
  platform?: string;
}

export function DnsConsentDialog({
  onAllow,
  onDecline,
  onClose,
  platform,
}: DnsConsentDialogProps) {
  const isWindows = platform === 'windows';
  return (
    <Modal title="System Website Blocking" onClose={onClose ?? onDecline}>
      <div className="dialog-stack">
        <p>
          <strong>
            Enforce website blocking across all browsers and apps.
          </strong>
        </p>
        <p className="muted" style={{ margin: '0.75rem 0', lineHeight: 1.5 }}>
          Preelock can enforce your website rules across all apps and browsers
          even if the browser extension is not connected.
        </p>
        <p className="muted" style={{ margin: '0 0 1rem 0', lineHeight: 1.5 }}>
          {isWindows
            ? 'This requires a one-time administrator approval to configure system-level website blocking. Your credentials are confirmed only in the native Windows dialog and are never seen or stored by Preelock.'
            : 'This installs Preelock’s privileged helper daemon to enforce SelfControl-grade website blocking (hosts + PF firewall) that even VPNs cannot bypass. Your admin password is asked once in the native macOS dialog and is never seen, stored, or logged.'}
        </p>
        <div
          className="row controls"
          style={{
            justifyContent: 'flex-end',
            gap: '0.75rem',
            marginTop: '1rem',
          }}
        >
          <button type="button" className="secondary" onClick={onDecline}>
            Cancel
          </button>
          <button type="button" onClick={onAllow}>
            {isWindows ? 'Enable Native Blocking' : 'Install Helper'}
          </button>
        </div>
      </div>
    </Modal>
  );
}
