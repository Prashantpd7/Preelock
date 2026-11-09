import { useEffect, useId, useRef, useState } from 'react';

export type DropdownOption = {
  value: string;
  label: string;
};

type DropdownProps = {
  /** Current value; must match one option's value, or '' for a placeholder option. */
  value: string;
  options: DropdownOption[];
  onChange: (value: string) => void;
  disabled?: boolean;
};

/**
 * Shared app-rendered dropdown replacing the native <select> elements.
 * The trigger keeps the native select's look (full width, 44px min-height,
 * 12px 13px padding with room for the chevron at right 12px center); only
 * the opened list is app-rendered, following the SessionCard "⋯" menu
 * pattern (fixed backdrop, panel anchored flush under the trigger).
 *
 * Accessibility uses the listbox/option pattern: the trigger keeps focus
 * while open and tracks the highlighted option with aria-activedescendant,
 * so focus is already on the trigger when the panel closes. The trigger
 * and every option are type="button", so choosing an option inside a
 * <form> never submits it.
 */
export function Dropdown({ value, options, onChange, disabled }: DropdownProps) {
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState<number | null>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const wrapRef = useRef<HTMLDivElement>(null);
  const rawId = useId().replace(/:/g, '');
  const listboxId = `dropdown-${rawId}-listbox`;
  const optionId = (index: number) => `dropdown-${rawId}-option-${index}`;

  const selectedIndex = options.findIndex((option) => option.value === value);
  const selectedLabel =
    selectedIndex >= 0 ? options[selectedIndex].label : '';
  const safeActive =
    activeIndex === null || options.length === 0
      ? null
      : Math.min(activeIndex, options.length - 1);

  useEffect(() => {
    if (!open || safeActive === null) return;
    try {
      optionRefs.current[safeActive]?.scrollIntoView({ block: 'nearest' });
    } catch {
      // jsdom and some embeds lack scrollIntoView; highlighting still works.
    }
  }, [open, safeActive]);

  function openWithActive(index: number | null) {
    if (disabled || options.length === 0) return;
    setActiveIndex(index);
    setOpen(true);
  }

  function close(focusTrigger: boolean) {
    setOpen(false);
    setActiveIndex(null);
    if (focusTrigger) triggerRef.current?.focus();
  }

  function select(index: number) {
    const option = options[index];
    if (!option) return;
    onChange(option.value);
    close(true);
  }

  function onTriggerKeyDown(e: React.KeyboardEvent) {
    if (disabled) return;
    if (!open) {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        openWithActive(selectedIndex >= 0 ? selectedIndex : 0);
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        openWithActive(
          selectedIndex >= 0 ? selectedIndex : options.length - 1,
        );
      } else if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        openWithActive(selectedIndex >= 0 ? selectedIndex : 0);
      }
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActiveIndex((current) =>
        Math.min((current ?? 0) + 1, options.length - 1),
      );
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActiveIndex((current) => Math.max((current ?? 0) - 1, 0));
    } else if (e.key === 'Home') {
      e.preventDefault();
      setActiveIndex(0);
    } else if (e.key === 'End') {
      e.preventDefault();
      setActiveIndex(options.length - 1);
    } else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      if (safeActive !== null) select(safeActive);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      close(true);
    }
  }

  return (
    <div className="dropdown-wrap" ref={wrapRef}>
      <button
        type="button"
        className="dropdown-trigger"
        ref={triggerRef}
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listboxId : undefined}
        aria-activedescendant={
          open && safeActive !== null ? optionId(safeActive) : undefined
        }
        onClick={() => {
          if (open) close(false);
          else openWithActive(selectedIndex >= 0 ? selectedIndex : 0);
        }}
        onKeyDown={onTriggerKeyDown}
        onBlur={(e) => {
          if (!wrapRef.current?.contains(e.relatedTarget as Node | null)) {
            close(false);
          }
        }}
      >
        <span className="dropdown-trigger-label">{selectedLabel}</span>
      </button>
      {open && (
        <>
          <div
            className="dropdown-backdrop"
            onClick={() => close(true)}
          />
          <div
            className="dropdown-panel"
            role="listbox"
            id={listboxId}
            aria-orientation="vertical"
          >
            {options.map((option, index) => (
              <button
                type="button"
                key={option.value}
                id={optionId(index)}
                ref={(el) => {
                  optionRefs.current[index] = el;
                }}
                className={[
                  'dropdown-option',
                  index === selectedIndex ? 'selected' : '',
                  index === safeActive ? 'active' : '',
                ]
                  .filter(Boolean)
                  .join(' ')}
                role="option"
                aria-selected={index === selectedIndex}
                tabIndex={-1}
                onClick={() => select(index)}
                onMouseEnter={() => setActiveIndex(index)}
              >
                {option.label}
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
