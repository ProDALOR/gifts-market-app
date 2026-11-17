import { useEffect, useId, useRef, useState } from "react";
import type { ReactNode } from "react";

/** Compact select with consistent rendering, outside dismissal and keyboard navigation. */
export function SelectPicker<T extends string>({
  label,
  className,
  options,
  value,
  change,
  render,
}: {
  label: string;
  className: string;
  options: readonly { value: T; label: string }[];
  value: T;
  change: (value: T) => void;
  render?: (value: T) => ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const host = useRef<HTMLDivElement>(null),
    trigger = useRef<HTMLButtonElement>(null);
  const id = useId(),
    disabled = options.length < 2;
  const selected = options.find((option) => option.value === value);
  useEffect(() => {
    if (!open) return;
    host.current?.querySelector<HTMLElement>('[aria-selected="true"]')?.focus();
    const outside = (event: PointerEvent) => {
      if (!host.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);
  function close() {
    setOpen(false);
    trigger.current?.focus();
  }
  return (
    <div
      className={`select-picker ${className}`}
      ref={host}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape" && open) {
          event.preventDefault();
          event.stopPropagation();
          close();
        } else if (
          !disabled &&
          ["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)
        ) {
          event.preventDefault();
          if (!open) {
            setOpen(true);
            return;
          }
          const items = Array.from(
            host.current?.querySelectorAll<HTMLButtonElement>(
              '[role="option"]',
            ) || [],
          );
          const current = items.indexOf(
            document.activeElement as HTMLButtonElement,
          );
          items[
            event.key === "Home"
              ? 0
              : event.key === "End"
                ? items.length - 1
                : (current +
                    (event.key === "ArrowUp" ? -1 : 1) +
                    items.length) %
                  items.length
          ]?.focus();
        }
      }}
    >
      <button
        ref={trigger}
        className="picker-trigger"
        role="combobox"
        aria-label={label}
        aria-expanded={open && !disabled}
        aria-haspopup="listbox"
        aria-controls={id}
        title={selected?.label}
        disabled={disabled}
        onClick={() => setOpen(!open)}
      >
        {render ? render(value) : <span>{selected?.label || value}</span>}
        {!disabled && (
          <svg
            className="picker-chevron"
            viewBox="0 0 16 16"
            aria-hidden="true"
          >
            <path d="m4 6 4 4 4-4" />
          </svg>
        )}
      </button>
      {open && !disabled && (
        <div
          id={id}
          className="picker-options"
          role="listbox"
          aria-label={`${label} options`}
        >
          {options.map((option) => (
            <button
              key={option.value}
              role="option"
              title={option.label}
              aria-label={option.label}
              aria-selected={option.value === value}
              tabIndex={option.value === value ? 0 : -1}
              onClick={() => {
                change(option.value);
                close();
              }}
            >
              {render ? render(option.value) : <span>{option.label}</span>}
              {option.value === value && (
                <span className="picker-check" aria-hidden="true">
                  ✓
                </span>
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
