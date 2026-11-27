import { useHistorySlot, useNavigationBack } from "../app/navigation";
import { CloseIcon } from "./icons";
import { useLayoutEffect, useRef } from "react";
import type { ReactNode } from "react";
import { usePreviewEnvironment } from "../previews/preview-environment";
import { useSheetSwipe } from "./use-sheet-swipe";

export function Modal({
  close,
  children,
  label,
  describedBy,
  className,
  heading,
  closeLabel,
  viewportClose = false,
  swipeToClose = false,
  scrollKey,
}: {
  close: () => void;
  children: ReactNode;
  label: string;
  describedBy?: string;
  className: string;
  heading?: string;
  closeLabel: string;
  viewportClose?: boolean;
  swipeToClose?: boolean;
  scrollKey?: string;
}) {
  const scrolls = useHistorySlot<
    Map<string, { sheet: number; content: number }>
  >("modal.scrolls", () => new Map());
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
    const swipe = element.dataset.swipeDismiss === "true";
    const sheet = swipe
      ? element.querySelector<HTMLElement>(":scope > .modal-sheet")
      : null;
    const target = sheet ?? element;
    const exit = target.animate(
      [
        {
          opacity: 1,
          ...(sheet
            ? { transform: getComputedStyle(sheet).transform }
            : viewportClose
              ? {}
              : { transform: "translateY(0)" }),
        },
        {
          opacity: 0,
          ...(sheet
            ? { transform: `translate3d(0, ${element.offsetHeight + 24}px, 0)` }
            : viewportClose
              ? {}
              : { transform: "translateY(8px)" }),
        },
      ],
      {
        duration: swipe ? 180 : 140,
        easing: swipe ? "cubic-bezier(.2,.7,.2,1)" : "ease-in",
        fill: "forwards",
      },
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
    if (element && scrollKey) {
      const saved = scrolls.current.get(scrollKey);
      if (saved) {
        element.scrollTop = saved.sheet;
        const content = element.querySelector(".detail-content");
        if (content) content.scrollTop = saved.content;
      }
    }
    if (element?.animate && motion) {
      const enter = element.animate(
        [
          {
            opacity: 0,
            ...(viewportClose ? {} : { transform: "translateY(12px)" }),
          },
          {
            opacity: 1,
            ...(viewportClose ? {} : { transform: "translateY(0)" }),
          },
        ],
        { duration: 180, easing: "cubic-bezier(.2,.7,.2,1)" },
      );
      enter.id = "modal-enter";
      animation.current = enter;
    }
    return () => {
      if (element && scrollKey) {
        scrolls.current.delete(scrollKey);
        scrolls.current.set(scrollKey, {
          sheet: element.scrollTop,
          content: element.querySelector(".detail-content")?.scrollTop ?? 0,
        });
        while (scrolls.current.size > 4)
          scrolls.current.delete(scrolls.current.keys().next().value!);
      }
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
  useNavigationBack(true, dismiss, 20);
  useSheetSwipe(
    dialog,
    swipeToClose,
    motion,
    () => phase.current === "open",
    dismiss,
  );
  const closeButton = (
    <button
      className="modal-close"
      aria-label={closeLabel}
      onClick={dismiss}
      autoFocus
    >
      <CloseIcon />
    </button>
  );
  return (
    <dialog
      ref={dialog}
      className={`modal ${className}${viewportClose ? " modal-viewport-close" : ""}${swipeToClose ? " modal-swipe" : ""}`}
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
      {swipeToClose ? <div className="modal-sheet">{children}</div> : children}
    </dialog>
  );
}
