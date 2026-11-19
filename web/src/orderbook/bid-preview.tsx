import type { CatalogOption } from "../market";
import { traits, type BidCriteria } from "../orderbook";
import { TraitComposition } from "../previews/trait-composition";

const PREVIEW_CHOICES = 4;

/** The picker's composition, bounded for a static listing card. */
export function BidPreview({
  criteria,
  options,
}: {
  criteria: BidCriteria;
  options: ReadonlyMap<string, CatalogOption>;
}) {
  const groups = traits.map((trait) => {
    const ids = criteria[trait.id];
    const available = ids.flatMap((id) => {
      const option = options.get(id);
      return option?.kind === trait.kind ? [option] : [];
    });
    return {
      ...trait,
      total: ids.length,
      available,
      shown: available.slice(0, PREVIEW_CHOICES),
      missing: ids.length - available.length,
    };
  });
  const [models, backdrops, patterns] = groups;
  const label = groups
    .map(
      (group) =>
        `${group.label}: ${
          group.total
            ? [
                ...group.available.map((option) => option.name),
                ...(group.missing ? [`${group.missing} unavailable`] : []),
              ].join(", ")
            : "any"
        }`,
    )
    .join("; ");
  return (
    <TraitComposition
      models={models.shown}
      backdrops={backdrops.shown}
      patterns={patterns.shown}
      compact
      label={`Bid alternatives. ${label}`}
    >
      {!models.shown.length && (
        <span className="bid-preview-empty">
          {models.total ? "Unavailable model" : "Any model"}
        </span>
      )}
      <div className="bid-preview-summary">
        {groups.flatMap((group) => {
          const extra = group.available.length - group.shown.length;
          return [
            ...(extra
              ? [
                  <span key={`${group.id}-more`}>
                    +{extra} {group.label.toLowerCase()}s
                  </span>,
                ]
              : []),
            ...(group.missing
              ? [
                  <span key={`${group.id}-missing`}>
                    {group.missing} unavailable {group.label.toLowerCase()}s
                  </span>,
                ]
              : []),
          ];
        })}
      </div>
    </TraitComposition>
  );
}
