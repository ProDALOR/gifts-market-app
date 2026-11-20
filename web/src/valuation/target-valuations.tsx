import { useEffect, useRef, useState } from "react";
import type {
  CatalogOption,
  Collection,
  TraitFilters,
  ValuationQuery,
  ValuationResponse,
  ValuationStrategy,
} from "../market";
import { valuation } from "../market";
import { Modal } from "../ui/modal";
import { ValuationFields } from "./display";
import { strategyClient } from "./strategies";
import { playbackBudget } from "../previews/playback-budget";
import { useMarketStatus } from "../use-market-status";
import { liveSocket } from "../live-socket";
import { estimateIsStale } from "./freshness";
import { usePreviewEnvironment } from "../previews/preview-environment";
import "./target-valuations.css";
type Kind = "collection" | "model" | "backdrop" | "model_backdrop";
type Props = {
  collection: Collection;
  currency: string;
  options: ReadonlyMap<string, CatalogOption>;
  filters: TraitFilters;
};
export function TargetValuations(props: Props) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        className="target-values-trigger"
        onClick={() => setOpen(true)}
        aria-haspopup="dialog"
      >
        Collection & trait values <span aria-hidden="true">↗</span>
      </button>
      {open && <ValuationPanel {...props} close={() => setOpen(false)} />}
    </>
  );
}
function localTime(date: Date) {
  return new Date(date.getTime() - date.getTimezoneOffset() * 60000)
    .toISOString()
    .slice(0, 16);
}
function ValuationPanel({
  collection,
  currency,
  options,
  filters,
  close,
}: Props & { close: () => void }) {
  const models = [...options.values()].filter((o) => o.kind === "model");
  const backdrops = [...options.values()].filter((o) => o.kind === "backdrop");
  const [kind, setKind] = useState<Kind>(
    filters.models?.length === 1
      ? filters.backdrops?.length === 1
        ? "model_backdrop"
        : "model"
      : filters.backdrops?.length === 1
        ? "backdrop"
        : "collection",
  );
  const [model, setModel] = useState(
    filters.models?.[0] || models[0]?.id || "",
  );
  const [backdrop, setBackdrop] = useState(
    filters.backdrops?.[0] || backdrops[0]?.id || "",
  );
  const [from, setFrom] = useState(() =>
    localTime(new Date(Date.now() - 30 * 86400000)),
  );
  const [until, setUntil] = useState(() => localTime(new Date()));
  const [strategies, setStrategies] = useState<ValuationStrategy[]>();
  const [selected, setSelected] = useState<string[]>([]);
  const [metadataAttempt, setMetadataAttempt] = useState(0);
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const [report, setReport] = useState<{
    value: ValuationResponse;
    generation: number;
  }>();
  const request = useRef<AbortController | null>(null);
  const results = useRef<HTMLElement>(null);
  const { pageVisible, reducedMotion } = usePreviewEnvironment();
  const status = useMarketStatus(true);
  useEffect(() => playbackBudget.suspendAll(), []);
  useEffect(() => () => request.current?.abort(), []);
  useEffect(() => {
    if (report && pageVisible)
      results.current?.scrollIntoView({
        block: "start",
        behavior: reducedMotion ? "instant" : "smooth",
      });
  }, [report]);
  useEffect(() => {
    const stop = new AbortController();
    void strategyClient
      .list(stop.signal)
      .then((rows) => {
        if (stop.signal.aborted) return;
        setStrategies(rows);
        setSelected(
          rows.filter((row) => row.targets.includes(kind)).map((row) => row.id),
        );
        setError("");
      })
      .catch((cause) => {
        if (!stop.signal.aborted)
          setError(
            cause instanceof Error
              ? cause.message
              : "Strategy information unavailable",
          );
      });
    return () => stop.abort();
  }, [metadataAttempt]);
  const available =
    strategies?.filter((row) => row.targets.includes(kind)) || [];
  const needsPeriod = available.some(
    (row) => selected.includes(row.id) && row.requires_period,
  );
  const needsModel = kind === "model" || kind === "model_backdrop";
  const needsBackdrop = kind === "backdrop" || kind === "model_backdrop";
  function clear() {
    request.current?.abort();
    request.current = null;
    setReport(undefined);
    setPending(false);
    setError("");
  }
  async function calculate() {
    clear();
    const stop = new AbortController();
    request.current = stop;
    const generation = liveSocket.statusSnapshot().generation;
    const target: ValuationQuery["target"] =
      kind === "model_backdrop"
        ? {
            kind,
            collection_id: collection.id,
            model_id: model,
            backdrop_id: backdrop,
          }
        : kind === "model"
          ? { kind, collection_id: collection.id, model_id: model }
          : kind === "backdrop"
            ? { kind, collection_id: collection.id, backdrop_id: backdrop }
            : { kind, collection_id: collection.id };
    setPending(true);
    try {
      if (
        needsPeriod &&
        (!from ||
          !until ||
          !Number.isFinite(Date.parse(from)) ||
          !Number.isFinite(Date.parse(until)) ||
          new Date(from) >= new Date(until))
      )
        throw new Error("Choose a valid execution period.");
      const period = needsPeriod
        ? {
            from: new Date(from).toISOString(),
            until: new Date(until).toISOString(),
          }
        : null;
      const value = await valuation(
        {
          target,
          currency_id: currency,
          strategies: selected.filter((id) =>
            available.some((row) => row.id === id),
          ),
          period,
        },
        stop.signal,
      );
      if (!stop.signal.aborted) setReport({ value, generation });
    } catch (cause) {
      if (!stop.signal.aborted)
        setError(
          cause instanceof Error ? cause.message : "Valuations unavailable",
        );
    } finally {
      if (!stop.signal.aborted) setPending(false);
    }
  }
  return (
    <Modal
      className="valuation-popup target-values"
      label="Collection and trait values"
      heading={collection.name}
      close={close}
      closeLabel="Close valuations"
    >
      <div className="valuation-popup-body">
        <label>
          Value
          <select
            aria-label="Value"
            value={kind}
            onChange={(event) => {
              clear();
              setKind(event.target.value as Kind);
            }}
          >
            <option value="collection">Collection</option>
            <option value="model" disabled={!models.length}>
              Model
            </option>
            <option value="backdrop" disabled={!backdrops.length}>
              Backdrop
            </option>
            <option
              value="model_backdrop"
              disabled={!models.length || !backdrops.length}
            >
              Model + backdrop
            </option>
          </select>
        </label>
        {needsModel && (
          <label>
            Model
            <select
              aria-label="Model"
              value={model}
              onChange={(e) => {
                clear();
                setModel(e.target.value);
              }}
            >
              {models.map((o) => (
                <option key={o.id} value={o.id}>
                  {o.name}
                </option>
              ))}
            </select>
          </label>
        )}
        {needsBackdrop && (
          <label>
            Backdrop
            <select
              aria-label="Backdrop"
              value={backdrop}
              onChange={(e) => {
                clear();
                setBackdrop(e.target.value);
              }}
            >
              {backdrops.map((o) => (
                <option key={o.id} value={o.id}>
                  {o.name}
                </option>
              ))}
            </select>
          </label>
        )}
        <fieldset>
          <legend>Strategies</legend>
          {available.map((row) => (
            <label className="target-strategy" key={row.id}>
              <input
                type="checkbox"
                checked={selected.includes(row.id)}
                onChange={(e) => {
                  clear();
                  setSelected((previous) =>
                    e.target.checked
                      ? [...previous, row.id]
                      : previous.filter((id) => id !== row.id),
                  );
                }}
              />
              <span>
                <strong>{row.name}</strong>
                <small>{row.description}</small>
              </span>
            </label>
          ))}
        </fieldset>
        {!strategies && !error && <p role="status">Loading strategies…</p>}
        {needsPeriod && (
          <fieldset>
            <legend>Observed execution period</legend>
            <label>
              From
              <input
                type="datetime-local"
                value={from}
                onChange={(e) => {
                  clear();
                  setFrom(e.target.value);
                }}
              />
            </label>
            <label>
              Until (exclusive)
              <input
                type="datetime-local"
                value={until}
                onChange={(e) => {
                  clear();
                  setUntil(e.target.value);
                }}
              />
            </label>
            <p className="valuation-note">
              Available history may not cover this entire period.
            </p>
          </fieldset>
        )}
        {error && (
          <p role="alert" className="valuation-guide-error">
            {error}
            {!strategies && (
              <button onClick={() => setMetadataAttempt((x) => x + 1)}>
                Retry strategy information
              </button>
            )}
          </p>
        )}
        <button
          className="primary"
          onClick={() => void calculate()}
          disabled={
            pending ||
            !available.some((row) => selected.includes(row.id)) ||
            (needsModel && !model) ||
            (needsBackdrop && !backdrop)
          }
        >
          {pending ? "Calculating…" : "Calculate values"}
        </button>
        {report && (
          <section
            ref={results}
            className="target-results"
            aria-label="Valuation results"
          >
            <p className="valuation-note">
              {estimateIsStale(report.value, report.generation, status)
                ? "Market updates changed or are delayed; recalculate to refresh."
                : `Calculated ${new Date(report.value.evaluated_at).toLocaleString()}`}{" "}
              · {currency}
            </p>
            {report.value.results.map((row) => (
              <article key={row.strategy}>
                <h3>
                  {strategies?.find((item) => item.id === row.strategy)?.name ||
                    row.strategy}
                </h3>
                <ValuationFields fields={row.display.fields} />
                {row.display.summary && (
                  <p className="valuation-note">{row.display.summary}</p>
                )}
                <p className="valuation-note">{row.display.note}</p>
              </article>
            ))}
          </section>
        )}
      </div>
    </Modal>
  );
}
