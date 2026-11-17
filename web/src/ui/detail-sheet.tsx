import { useEffect, useRef } from "react";
import type { ReactNode } from "react";
export function DetailSheet({
  close,
  children,
}: {
  close: () => void;
  children: ReactNode;
}) {
  const dialog = useRef<HTMLDialogElement>(null),
    closeAction = useRef(close);
  closeAction.current = close;
  useEffect(() => {
    const element = dialog.current;
    const previous = document.activeElement;
    element?.showModal();
    return () => {
      element?.close();
      if (previous instanceof HTMLElement && previous.isConnected)
        previous.focus({ preventScroll: true });
    };
  }, []);
  return (
    <dialog
      ref={dialog}
      className="detail"
      aria-label="Order details"
      aria-describedby="detail-title"
      onCancel={(event) => {
        event.preventDefault();
        closeAction.current();
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
          closeAction.current();
      }}
    >
      {children}
    </dialog>
  );
}
