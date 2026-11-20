import type { CatalogOption } from "../market";
import type { BidCriteria } from "../orderbook";
import { resolveTraits } from "../catalog/traits";
import { TraitComposition } from "../previews/trait-composition";

/** The picker's composition, bounded for static cards and expanded details. */
export function BidPreview({
  criteria,
  options,
  names,
  compact = true,
}: {
  criteria: BidCriteria;
  options: ReadonlyMap<string, CatalogOption>;
  names?: ReadonlyMap<string, string>;
  compact?: boolean;
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
      showModelNames={compact}
      playback="static"
      label={`Bid alternatives. ${label}`}
      header={
        compact && !backdrops.entries.length ? (
          <span className="bid-preview-backdrop">Any backdrop</span>
        ) : undefined
      }
      footer={
        badges.length ? (
          <div className="bid-preview-summary">
            {badges.map((badge) => (
              <span key={badge.text} title={badge.title}>
                {badge.text}
              </span>
            ))}
          </div>
        ) : undefined
      }
    >
      {!models.shown.length && (
        <span className="bid-preview-empty">
          {models.entries.length ? "Unavailable model" : "Any model"}
        </span>
      )}
    </TraitComposition>
  );
}
