import { lazy, Suspense, useEffect, useState } from "react";
import { guideKey } from "../valuation/strategies";
import type { OrderValuationState } from "../valuation/use-order-valuation";
import { Modal } from "../ui/modal";
import { ValuationFields } from "../valuation/display";
import { ValuationStatus } from "./valuation-status";
import { playbackBudget } from "../previews/playback-budget";
const ValuationMethodology = lazy(() => import("./valuation-methodology"));

/** Open the report’s guide on intent; closing aborts pending work. */
export function ValuationInfo({ state }: { state: OrderValuationState }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        className="valuation-info-button"
        aria-label="How is this calculated?"
        aria-haspopup="dialog"
        aria-expanded={open}
        disabled={!state.report}
        onClick={() => setOpen(true)}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <circle cx="12" cy="12" r="8.5" />
          <path d="M12 11v6m0-10v1" />
        </svg>
      </button>
      {open && <CalculationPopup state={state} close={() => setOpen(false)} />}
    </>
  );
}

/** Pending guide work and playback suspension share the popup lifetime. */
function CalculationPopup({
  state,
  close,
}: {
  state: OrderValuationState;
  close: () => void;
}) {
  useEffect(() => playbackBudget.suspendAll(), []);
  const strategy = state.strategy;
  return (
    <Modal
      className="valuation-popup"
      label="Calculation guide"
      close={close}
      closeLabel="Close calculation guide"
      heading={strategy?.name ?? "Calculation guide"}
    >
      <div className="valuation-popup-body">
        <section aria-label="Valuation result" aria-busy={state.loading}>
          <ValuationFields fields={state.result?.display.fields ?? []} />
          {state.result?.display.summary && (
            <p className="valuation-note">{state.result.display.summary}</p>
          )}
          <ValuationStatus state={state} detailed />
        </section>
        {strategy ? (
          <>
            <p className="valuation-note">{strategy.description}</p>
            <Suspense
              fallback={
                <p className="valuation-note" role="status">
                  Loading calculation guide…
                </p>
              }
            >
              <ValuationMethodology
                key={guideKey(strategy)}
                strategy={strategy}
              />
            </Suspense>
          </>
        ) : (
          <p className="valuation-note" role="status">
            Strategy information is unavailable.
          </p>
        )}
      </div>
    </Modal>
  );
}
