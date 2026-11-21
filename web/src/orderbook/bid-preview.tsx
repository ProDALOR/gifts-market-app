import type { CatalogOption } from "../market";
import type { ReactNode } from "react";
import type { BidCriteria } from "../orderbook";
import { resolveTraits } from "../catalog/traits";
import { TraitComposition } from "../previews/trait-composition";
import { UnrestrictedTrait } from "../previews/unrestricted-trait";

/** Static cards and animated details share the picker's composition and playback budget. */
export function BidPreview({
  criteria,
  options,
  names,
  compact = true,
  caption,
}: {
  criteria: BidCriteria;
  options: ReadonlyMap<string, CatalogOption>;
  names?: ReadonlyMap<string, string>;
  compact?: boolean;
  caption?: ReactNode;
}) {
  const groups = resolveTraits(criteria, options, names).map((group) => ({
    ...group,
    shown: group.available.slice(0, compact ? 4 : 16),
  }));
  const [models, backdrops, patterns] = groups;
  const label = groups
    .map(
      (group) =>
        `${group.label}: ${
          group.entries.length
            ? group.entries
                .map((entry) =>
                  entry.option ? entry.name : `${entry.name} (unavailable)`,
                )
                .join(", ")
            : "any"
        }`,
    )
    .join("; ");
  const overflow = groups.flatMap((group) => {
    const count = group.available.length - group.shown.length;
    return count
      ? [
          {
            count,
            text: `+${count} ${group.label.toLowerCase()}${count === 1 ? "" : "s"}`,
          },
        ]
      : [];
  });
  const missing = groups.flatMap((group) =>
    group.missing
      ? [
          {
            count: group.missing,
            text: `${group.missing} unavailable ${group.label.toLowerCase()}${group.missing === 1 ? "" : "s"}`,
          },
        ]
      : [],
  );
  const badges = [overflow, missing].flatMap((items, index) => {
    if (!items.length) return [];
    const title = items.map((item) => item.text).join(", ");
    if (compact && items.length > 1) {
      const total = items.reduce((sum, item) => sum + item.count, 0);
      return [
        {
          text: index === 0 ? `+${total} choices` : `${total} unavailable`,
          title,
        },
      ];
    }
    return items.map((item) => ({ text: item.text, title }));
  });
  return (
    <TraitComposition
      models={models.shown}
      backdrops={backdrops.shown}
      patterns={patterns.shown}
      compact={compact}
      playback={compact ? "static" : "detail"}
      label={`Bid alternatives. ${label}`}
      header={
        !backdrops.entries.length || !patterns.entries.length ? (
          <div className="bid-preview-unrestricted">
            {!backdrops.entries.length && <UnrestrictedTrait kind="backdrop" />}
            {!patterns.entries.length && <UnrestrictedTrait kind="pattern" />}
          </div>
        ) : undefined
      }
      footer={
        badges.length || caption ? (
          <>
            {badges.length > 0 && (
              <div className="bid-preview-summary">
                {badges.map((badge) => (
                  <span key={badge.text} title={badge.title}>
                    {badge.text}
                  </span>
                ))}
              </div>
            )}
            {caption}
          </>
        ) : undefined
      }
    >
      {!models.shown.length && (
        <span className="bid-preview-empty">
          {models.entries.length ? (
            "Unavailable model"
          ) : (
            <UnrestrictedTrait kind="model" />
          )}
        </span>
      )}
    </TraitComposition>
  );
}
