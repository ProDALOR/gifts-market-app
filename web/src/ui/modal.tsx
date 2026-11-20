import { useLayoutEffect, useRef } from "react";
import type { ReactNode } from "react";
import { usePreviewEnvironment } from "../previews/preview-environment";

export function Modal({
  close,
  children,
  label,
  describedBy,
  className,
  heading,
  closeLabel,
  viewportClose = false,
}: {
  close: () => void;
  children: ReactNode;
  label: string;
  describedBy?: string;
  className: string;
  heading?: string;
  closeLabel: string;
  viewportClose?: boolean;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const closeAction = useRef(close);
  const animation = useRef<Animation | undefined>(undefined);
  const phase = useRef<"open" | "closing" | "closed" | "unmounted">(
    "unmounted",
  );
  const { pageVisible, reducedMotion } = usePreviewEnvironment();
  const motion = pageVisible && !reducedMotion;
  closeAction.current = close;
  const finish = () => {
    if (phase.current !== "closing") return;
    phase.current = "closed";
    closeAction.current();
  };
  const dismiss = () => {
    const element = dialog.current;
    if (!element || phase.current !== "open") return;
    phase.current = "closing";
    animation.current?.cancel();
    if (!motion || !element.animate) return finish();
    element.dataset.closing = "true";
    const exit = element.animate(
      [
        { opacity: 1, transform: viewportClose ? "none" : "translateY(0)" },
        { opacity: 0, transform: viewportClose ? "none" : "translateY(8px)" },
      ],
      { duration: 140, easing: "ease-in", fill: "forwards" },
    );
    exit.id = "modal-exit";
    animation.current = exit;
    void exit.finished.then(finish, () => {});
  };
  useLayoutEffect(() => {
    const element = dialog.current;
    const previous = document.activeElement;
    phase.current = "open";
    element?.showModal();
    if (element?.animate && motion) {
      const enter = element.animate(
        [
          {
            opacity: 0,
            transform: viewportClose ? "none" : "translateY(12px)",
          },
          { opacity: 1, transform: viewportClose ? "none" : "translateY(0)" },
        ],
        { duration: 180, easing: "cubic-bezier(.2,.7,.2,1)" },
      );
      enter.id = "modal-enter";
      animation.current = enter;
    }
    return () => {
      phase.current = "unmounted";
      animation.current?.cancel();
      element?.close();
      if (previous instanceof HTMLElement && previous.isConnected)
        previous.focus({ preventScroll: true });
    };
  }, []);
  useLayoutEffect(() => {
    if (!motion) {
      animation.current?.cancel();
      if (phase.current === "closing") finish();
    }
  }, [pageVisible, reducedMotion]);
  const closeButton = (
    <button
      className="modal-close"
      aria-label={closeLabel}
      onClick={dismiss}
      autoFocus
    >
      ×
    </button>
  );
  return (
    <dialog
      ref={dialog}
      className={`modal ${className}${viewportClose ? " modal-viewport-close" : ""}`}
      aria-label={label}
      aria-describedby={describedBy}
      onCancel={(event) => {
        event.stopPropagation();
        event.preventDefault();
        dismiss();
      }}
      onClickCapture={(event) => {
        if (phase.current === "closing") {
          event.preventDefault();
          event.stopPropagation();
        }
      }}
      onClick={(event) => {
        if (event.target !== event.currentTarget) return;
        const box = event.currentTarget.getBoundingClientRect();
        if (
          event.clientY < box.top ||
          event.clientY > box.bottom ||
          event.clientX < box.left ||
          event.clientX > box.right
        )
          dismiss();
      }}
    >
      {heading ? (
        <header className="modal-heading">
          <h3>{heading}</h3>
          {closeButton}
        </header>
      ) : (
        closeButton
      )}
      {children}
    </dialog>
  );
}
