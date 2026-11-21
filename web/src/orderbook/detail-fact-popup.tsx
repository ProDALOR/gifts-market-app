import { useEffect } from "react";
import type { ReactNode } from "react";
import { Modal } from "../ui/modal";
import { playbackBudget } from "../previews/playback-budget";

/** Small read-only details pause artwork and share modal focus/dismissal behavior. */
export function DetailFactPopup({
  heading,
  label,
  close,
  children,
}: {
  heading: string;
  label: string;
  close: () => void;
  children: ReactNode;
}) {
  useEffect(() => playbackBudget.suspendAll(), []);
  return (
    <Modal
      className="detail-fact-popup"
      heading={heading}
      label={label}
      closeLabel={`Close ${label.toLowerCase()}`}
      close={close}
    >
      <div className="detail-fact-content">{children}</div>
    </Modal>
  );
}
