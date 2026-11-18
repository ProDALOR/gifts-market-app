import type { ReactNode } from "react";
import { Modal } from "./modal";
export function DetailSheet({
  close,
  children,
}: {
  close: () => void;
  children: ReactNode;
}) {
  return (
    <Modal
      close={close}
      className="detail"
      label="Order details"
      closeLabel="Close details"
      describedBy="detail-title"
    >
      {children}
    </Modal>
  );
}
