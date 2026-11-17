import { collectionOptions } from "../catalog/selectors";
import { memo, useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import type { TraitFilters, TraitKey } from "../market";
import { GiftArtwork } from "./gift-artwork";
import { GiftBackdrop } from "./gift-backdrop";
import { backdropPalette } from "./gift-appearance";
import { balancedRows, balancedGrid, previewLayout } from "./preview-layout";
import {
  observePreviewSize,
  usePreviewVisibility,
} from "./preview-environment";
import type { CatalogOption, Collection } from "../orderbook";
// Render only explicitly selected catalog traits.
export const TraitPreview = memo(function TraitPreview({
  collection,
  options,
  filters,
  reset,
}: {
  collection?: Collection;
  options: ReadonlyMap<string, CatalogOption>;
  filters: TraitFilters;
  reset: () => void;
}) {
  const selected = (id: TraitKey) => {
    const ids = new Set(filters[id] || []);
    return collectionOptions(collection, id, options).filter((option) =>
      ids.has(option.id),
    );
  };
  const models = selected("models"),
    backdrops = selected("backdrops"),
    patterns = selected("patterns");
  if (!models.length && !backdrops.length && !patterns.length) return null;
  return (
    <SelectedTraitPreview
      models={models}
      backdrops={backdrops}
      patterns={patterns}
      reset={reset}
    />
  );
});
function SelectedTraitPreview({
  models,
  backdrops,
  patterns,
  reset,
}: {
  models: CatalogOption[];
  backdrops: CatalogOption[];
  patterns: CatalogOption[];
  reset: () => void;
}) {
  const host = useRef<HTMLElement>(null),
    visible = usePreviewVisibility(host);
  const [width, setWidth] = useState(358);
  useEffect(() => {
    const element = host.current;
    if (!element) return;
    const measure = () => setWidth(element.clientWidth);
    measure();
    return observePreviewSize(element, measure);
  }, []);
  const layout = previewLayout(
    width,
    models.length,
    backdrops.length,
    patterns.length,
  );
  const modelGrid = balancedGrid(models, layout.modelColumns),
    backdropRows = balancedRows(backdrops, layout.backdropColumns);
  const palette = backdropPalette(backdrops[0]) || {
    center: "#302d36",
    edge: "#302d36",
    pattern: "#c5afce",
  };
  return (
    <figure
      className="trait-preview"
      ref={host}
      aria-label="Virtual gift preview"
      style={
        {
          height: layout.height,
          "--preview-model-size": `${layout.modelSize}px`,
          "--preview-model-row-height": `${layout.modelRowHeight}px`,
        } as CSSProperties
      }
    >
      <div className="preview-bands" aria-hidden="true">
        {backdropRows.map((row, index) => (
          <div className="preview-backdrop-row" key={index}>
            {row.map((backdrop) => {
              const colors = backdropPalette(backdrop);
              return (
                <span className="preview-band" key={backdrop.id}>
                  {colors && (
                    <span
                      className="preview-gradient"
                      style={{
                        background: `radial-gradient(ellipse at 50% 42%, ${colors.center}, ${colors.edge})`,
                      }}
                    />
                  )}
                </span>
              );
            })}
          </div>
        ))}
      </div>
      {patterns.length > 0 && (
        <div className="preview-symbols" aria-hidden="true">
          <GiftBackdrop
            palette={palette}
            patternUrls={patterns.flatMap((option) =>
              option.kind === "pattern" && option.media
                ? [option.media.thumbnail_url]
                : [],
            )}
            visible={visible}
            layout="banner"
            background={false}
          />
        </div>
      )}
      {models.length > 0 && (
        <div
          className="preview-models"
          style={{
            gridTemplateColumns: `repeat(${modelGrid.columns}, minmax(0, 1fr))`,
            gridTemplateRows: `repeat(${modelGrid.rows}, minmax(var(--preview-model-row-height), 1fr))`,
          }}
        >
          {modelGrid.cells.map(({ item: model, row, column, span }) => (
            <div
              className="preview-model"
              key={model.id}
              style={{ gridRow: row, gridColumn: `${column} / span ${span}` }}
            >
              <GiftArtwork
                key={model.id}
                media={
                  model.kind === "model" ? model.media || undefined : undefined
                }
                label={model.name}
                fallback={model.name[0] || "◇"}
                playback="selected"
              />
              <span title={model.name}>{model.name}</span>
            </div>
          ))}
        </div>
      )}
      <button className="reset-traits" onClick={reset}>
        Reset traits
      </button>
    </figure>
  );
}
