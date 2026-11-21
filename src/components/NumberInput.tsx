import { useEffect, useRef, useState } from 'react';
import type { FocusEventHandler, InputHTMLAttributes } from 'react';

type NumberInputProps = Omit<
  InputHTMLAttributes<HTMLInputElement>,
  'type' | 'value' | 'onChange'
> & {
  value: number | string;
  onValueChange: (value: string) => void;
};

function normalizeNumberDraft(value: string): string {
  return value === '' ? '' : value.replace(/^0+(?=\d)/, '');
}

export function NumberInput({
  value,
  onValueChange,
  onFocus,
  onBlur,
  ...props
}: NumberInputProps) {
  const [draft, setDraft] = useState(() => normalizeNumberDraft(String(value)));
  const focused = useRef(false);

  useEffect(() => {
    if (!focused.current) {
      setDraft(normalizeNumberDraft(String(value)));
    }
  }, [value]);

  const handleFocus: FocusEventHandler<HTMLInputElement> = (event) => {
    focused.current = true;
    onFocus?.(event);
  };
  const handleBlur: FocusEventHandler<HTMLInputElement> = (event) => {
    focused.current = false;
    onBlur?.(event);
  };

  return (
    <input
      {...props}
      type="number"
      value={draft}
      onFocus={handleFocus}
      onBlur={handleBlur}
      onChange={(event) => {
        const next = normalizeNumberDraft(event.target.value);
        setDraft(next);
        onValueChange(next);
      }}
    />
  );
}
