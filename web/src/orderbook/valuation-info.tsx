import { lazy, Suspense, useEffect, useState } from "react";
import { guideKey } from "../valuation/strategies";
import type {
  OrderValuationState,
  OrderValuation,
} from "../valuation/use-order-valuation";
import { Modal } from "../ui/modal";
import { SharedIcon } from "../ui/icons";
import { ValuationFields } from "../valuation/display";
import { ValuationStatus } from "./valuation-status";
import { playbackBudget } from "../previews/playback-budget";
const ValuationMethodology = lazy(() => import("./valuation-methodology"));

/** Open the report’s guide on intent; closing aborts pending work. */
export function ValuationInfo({
  state,
  valuation,
  label = "How is this calculated?",
}: {
  state: OrderValuationState;
  valuation: OrderValuation;
  label?: string;
}) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        className="valuation-info-button"
        aria-label={label}
        aria-haspopup="dialog"
        aria-expanded={open}
        disabled={!state.report}
        onClick={() => setOpen(true)}
      >
        <SharedIcon name="info" />
      </button>
      {open && (
        <CalculationPopup
          state={state}
          valuation={valuation}
          close={() => setOpen(false)}
        />
      )}
    </>
  );
}

/** Pending guide work and playback suspension share the popup lifetime. */
function CalculationPopup({
  state,
  close,
  valuation,
}: {
  state: OrderValuationState;
  valuation: OrderValuation;
  close: () => void;
}) {
  useEffect(() => playbackBudget.suspendAll(), []);
  const { strategy, fields, evidence } = valuation;
  return (
    <Modal
      className="valuation-popup"
      label="Calculation guide"
      close={close}
      closeLabel="Close calculation guide"
      heading={strategy.name}
    >
      <div className="valuation-popup-body">
        <section aria-label="Valuation result" aria-busy={state.loading}>
          <ValuationFields fields={fields} />
          {evidence.length > 0 && (
            <p className="valuation-note">{evidence.join(" · ")}</p>
          )}
          <ValuationStatus state={state} valuation={valuation} detailed />
        </section>
        <p className="valuation-note">{strategy.description}</p>
        <Suspense
          fallback={
            <p className="valuation-note" role="status">
              Loading calculation guide…
            </p>
          }
        >
          <ValuationMethodology key={guideKey(strategy)} strategy={strategy} />
        </Suspense>
      </div>
    </Modal>
  );
}
