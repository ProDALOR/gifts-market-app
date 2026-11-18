import { lazy, Suspense, useEffect, useState } from "react";
import { useStrategy } from "../valuation/use-strategy";
import { guideKey } from "../valuation/strategies";
import type { GiftValuationState } from "../valuation/use-gift-valuation";
import { Modal } from "../ui/modal";
import { ValuationFields } from "../valuation/display";
import { ValuationStatus } from "./valuation-status";
import { playbackBudget } from "../previews/playback-budget";
const ValuationMethodology = lazy(() => import("./valuation-methodology"));

/** Discover and load the guide on intent; closing aborts pending work. */
export function ValuationInfo({ state }: { state: GiftValuationState }) {
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

/** Metadata, pending guide work and playback suspension share the popup lifetime. */
function CalculationPopup({
  state,
  close,
}: {
  state: GiftValuationState;
  close: () => void;
}) {
  useEffect(() => playbackBudget.suspendAll(), []);
  const {
    metadata: strategy,
    error,
    retry,
  } = useStrategy(state.report?.strategy);
  return (
    <Modal className="valuation-popup" label="Calculation guide" close={close}>
      <header className="valuation-popup-heading">
        <h3>{strategy?.name ?? "Calculation guide"}</h3>
        <button aria-label="Close calculation guide" onClick={close} autoFocus>
          ×
        </button>
      </header>
      <div className="valuation-popup-body">
        <section aria-label="Valuation result" aria-busy={state.loading}>
          <ValuationFields fields={state.fields.primary} />
          <ValuationStatus state={state} />
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
        ) : error ? (
          <div className="valuation-guide-error" role="alert">
            <p>{error}</p>
            <button onClick={retry}>Retry strategy information</button>
          </div>
        ) : (
          <p className="valuation-note" role="status">
            Loading strategy information…
          </p>
        )}
      </div>
    </Modal>
  );
}
