import { useState } from "react";
import { ExecutionAge } from "./execution-age";
import { DetailFactPopup } from "./detail-fact-popup";
import { fullTimestamp } from "../ui/time";
import { SharedIcon } from "../ui/icons";

export function ExecutionTime({ at }: { at: string }) {
  const [open, setOpen] = useState(false);
  const exact = fullTimestamp(at);
  if (!exact) return null;
  return (
    <>
      <button
        className="execution-time-trigger"
        onClick={() => setOpen(true)}
        aria-label="View execution time"
        aria-description={exact}
        aria-haspopup="dialog"
        aria-expanded={open}
        title={exact}
      >
        <SharedIcon name="history" />
        <ExecutionAge at={at} className="execution-time-age" ago />
      </button>
      {open && (
        <DetailFactPopup
          heading="Execution time"
          label="Execution time"
          close={() => setOpen(false)}
        >
          <time className="execution-time-exact" dateTime={at}>
            {exact}
          </time>
        </DetailFactPopup>
      )}
    </>
  );
}
