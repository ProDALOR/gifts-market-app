import type { CatalogOption } from "../market";
import { traits, giftNumberRange } from "../orderbook";
import type { BidCriteria } from "../orderbook";
import { OptionArt } from "../previews/option-art";

/** Present accepted alternatives; never compose them into a hypothetical gift. */
export function BidConditions({
  criteria,
  options,
  names,
  quantity,
  compact = false,
}: {
  criteria: BidCriteria;
  options: ReadonlyMap<string, CatalogOption>;
  names?: ReadonlyMap<string, string>;
  quantity?: number;
  compact?: boolean;
}) {
  const number = giftNumberRange(criteria.number);
  return (
    <dl
      className={`bid-conditions${compact ? " compact" : ""}`}
      aria-label="Bid conditions"
    >
      {traits.map((trait) => {
        const ids = criteria[trait.id];
        if (compact && trait.id !== "models" && !ids.length) return null;
        const shown = compact ? ids.slice(0, 1) : ids;
        return (
          <div key={trait.id} data-condition={trait.kind}>
            <dt>{trait.label}</dt>
            <dd>
              {!ids.length && (
                <span className="condition-any">
                  Any {trait.label.toLowerCase()}
                </span>
              )}
              {shown.map((id) => {
                const candidate = options.get(id);
                const option =
                  candidate?.kind === trait.kind ? candidate : undefined;
                const name =
                  option?.name ||
                  (!candidate ? names?.get(id) : undefined) ||
                  `Unavailable ${trait.label.toLowerCase()}`;
                return (
                  <span className="condition-chip" key={id} title={name}>
                    {option && <OptionArt option={option} />}
                    <span className="condition-name">{name}</span>
                  </span>
                );
              })}
              {shown.length < ids.length && (
                <span
                  className="condition-overflow"
                  aria-label={`${ids.length - shown.length} more ${trait.label.toLowerCase()} choices`}
                >
                  +{ids.length - shown.length}
                </span>
              )}
            </dd>
          </div>
        );
      })}
      {number && (
        <div data-condition="number">
          <dt>Gift #</dt>
          <dd>
            <span className="condition-range" title={`Gift # ${number}`}>
              #{number}
            </span>
          </dd>
        </div>
      )}
      {!compact && quantity != null && (
        <div data-condition="quantity">
          <dt>Quantity</dt>
          <dd>{quantity.toLocaleString()}</dd>
        </div>
      )}
    </dl>
  );
}
