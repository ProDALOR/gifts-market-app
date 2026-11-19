import { useEffect, useRef, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import { GiftArtwork } from "./gift-artwork";
import { GiftBackdrop } from "./gift-backdrop";
import { backdropPalette } from "./gift-appearance";
import { balancedRows, balancedGrid, previewLayout } from "./preview-layout";
import {
  observePreviewSize,
  usePreviewVisibility,
  usePreviewEnvironment,
} from "./preview-environment";
import type { CatalogOption } from "../market";
export function TraitComposition({
  models,
  backdrops,
  patterns,
  compact = false,
  label = "Virtual gift preview",
  children,
}: {
  models: CatalogOption[];
  backdrops: CatalogOption[];
  patterns: CatalogOption[];
  compact?: boolean;
  label?: string;
  children?: ReactNode;
}) {
  const host = useRef<HTMLElement>(null),
    visible = usePreviewVisibility(host);
  const { pageVisible } = usePreviewEnvironment();
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
  const columns = compact
    ? Math.min(2, models.length || 1)
    : layout.modelColumns;
  const modelSize = compact
    ? Math.min(
        models.length > 2 ? 24 : models.length > 1 ? 42 : 64,
        Math.max(24, width / columns - 20),
      )
    : layout.modelSize;
  const modelGrid = balancedGrid(models, columns),
    backdropRows = balancedRows(backdrops, layout.backdropColumns);
  const palette = backdropPalette(backdrops[0]) || {
    center: "#302d36",
    edge: "#302d36",
    pattern: "#c5afce",
  };
  return (
    <figure
      className={`trait-preview${compact ? " trait-preview-compact" : ""}`}
      ref={host}
      aria-label={label}
      style={
        {
          height: compact ? undefined : layout.height,
          "--preview-model-size": `${modelSize}px`,
          "--preview-model-row-height": `${compact ? modelSize + 14 : layout.modelRowHeight}px`,
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
            visible={visible && pageVisible}
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
                playback={compact ? "static" : "selected"}
              />
              <span title={model.name}>{model.name}</span>
            </div>
          ))}
        </div>
      )}
      {children}
    </figure>
  );
}
