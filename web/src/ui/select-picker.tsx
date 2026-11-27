import { useNavigationBack } from "../app/navigation";
import { useId, useLayoutEffect, useRef, useState } from "react";
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
  useLayoutEffect(() => {
    const element = host.current;
    const button = trigger.current;
    const menu = element?.querySelector<HTMLElement>('[role="listbox"]');
    if (!open || disabled || !element || !button || !menu) return;
    const place = () => {
      let top = 0,
        bottom = document.documentElement.clientHeight;
      // The sidebar is a clipping scrollport. Bound the menu to it as well as
      // the viewport, choosing a side instead of always opening upwards.
      for (
        let parent = element.parentElement;
        parent;
        parent = parent.parentElement
      ) {
        if (
          !/(auto|scroll|hidden|clip)/.test(getComputedStyle(parent).overflowY)
        )
          continue;
        const bounds = parent.getBoundingClientRect();
        const edge = bounds.top + parent.clientTop;
        top = Math.max(top, edge);
        bottom = Math.min(bottom, edge + parent.clientHeight);
      }
      const bounds = button.getBoundingClientRect();
      if (bounds.bottom <= top || bounds.top >= bottom) {
        setOpen(false);
        return false;
      }
      const below = Math.max(0, bottom - bounds.bottom - 4);
      const above = Math.max(0, bounds.top - top - 4);
      const upwards = below < menu.scrollHeight + 2 && above > below;
      menu.style.top = upwards ? "auto" : "calc(100% + 4px)";
      menu.style.bottom = upwards ? "calc(100% + 4px)" : "auto";
      menu.style.maxHeight = `${upwards ? above : below}px`;
      return true;
    };
    if (!place()) return;
    const selected = menu.querySelector<HTMLElement>('[aria-selected="true"]');
    selected?.focus({ preventScroll: true });
    if (selected)
      menu.scrollTop = Math.max(
        0,
        selected.offsetTop + selected.offsetHeight - menu.clientHeight,
      );
    const outside = (event: PointerEvent) => {
      if (!host.current?.contains(event.target as Node)) setOpen(false);
    };
    let frame: number | undefined;
    const schedule = (event: Event) => {
      if (event.target instanceof Node && menu.contains(event.target)) return;
      if (frame !== undefined) return;
      frame = requestAnimationFrame(() => {
        frame = undefined;
        place();
      });
    };
    document.addEventListener("pointerdown", outside);
    window.addEventListener("resize", schedule);
    window.addEventListener("scroll", schedule, true);
    return () => {
      document.removeEventListener("pointerdown", outside);
      window.removeEventListener("resize", schedule);
      window.removeEventListener("scroll", schedule, true);
      if (frame !== undefined) cancelAnimationFrame(frame);
    };
  }, [open, disabled]);
  function close() {
    setOpen(false);
    trigger.current?.focus({ preventScroll: true });
  }
  useNavigationBack(open && !disabled, close, 15);
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
