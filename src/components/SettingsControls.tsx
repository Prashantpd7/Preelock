import { useEffect, useRef, type ReactNode } from 'react';
import { X, Check } from 'lucide-react';
export function SettingRow({
  title,
  hint,
  children,
}: {
  title: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div className="preference-row">
      <div className="preference-label">
        <h3>{title}</h3>
        {hint && <p>{hint}</p>}
      </div>
      <div className="preference-control">{children}</div>
    </div>
  );
}
export function Toggle({
  label,
  checked,
  onChange,
  disabled = false,
}: {
  label: string;
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-label={label}
      aria-checked={checked}
      className="toggle"
      disabled={disabled}
      onClick={() => onChange(!checked)}
    >
      <span>{checked && <Check size={12} />}</span>
    </button>
  );
}
export function Choices<T extends string>({
  label,
  value,
  choices,
  onChange,
  disabled = false,
}: {
  label: string;
  value: T;
  choices: readonly (readonly [T, string])[];
  onChange: (value: T) => void;
  disabled?: boolean;
}) {
  return (
    <div className="segmented" role="group" aria-label={label}>
      {choices.map(([id, text]) => (
        <button
          type="button"
          key={id}
          disabled={disabled}
          aria-pressed={id === value}
          onClick={() => onChange(id)}
        >
          {text}
        </button>
      ))}
    </div>
  );
}
export function Modal({
  title,
  children,
  onClose,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (el?.showModal) {
      el.showModal();
    } else {
      el?.setAttribute('open', '');
    }
    return () => {
      if (el?.close) {
        el.close();
      } else {
        el?.removeAttribute('open');
      }
    };
  }, []);
  return (
    <dialog
      ref={ref}
      className="settings-dialog"
      aria-label={title}
      onCancel={onClose}
    >
      <div className="dialog-heading">
        <h2>{title}</h2>
        <button
          type="button"
          className="icon-button"
          aria-label="Close dialog"
          onClick={onClose}
        >
          <X size={20} />
        </button>
      </div>
      {children}
    </dialog>
  );
}
