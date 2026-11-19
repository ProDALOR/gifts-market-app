import type { CatalogOption } from "../market";
import { giftNumberRange } from "../orderbook";
import { resolveTraits } from "../catalog/traits";
import type { BidCriteria } from "../orderbook";
import { OptionArt } from "../previews/option-art";

/** Complete accepted alternatives for bid details. */
export function BidConditions({
  criteria,
  options,
  names,
  quantity,
}: {
  criteria: BidCriteria;
  options: ReadonlyMap<string, CatalogOption>;
  names?: ReadonlyMap<string, string>;
  quantity?: number;
}) {
  const number = giftNumberRange(criteria.number);
  return (
    <dl className="bid-conditions" aria-label="Bid conditions">
      {resolveTraits(criteria, options, names).map((trait) => {
        return (
          <div key={trait.id} data-condition={trait.kind}>
            <dt>{trait.label}</dt>
            <dd>
              {!trait.entries.length && (
                <span className="condition-any">
                  Any {trait.label.toLowerCase()}
                </span>
              )}
              {trait.entries.map(({ id, name, option }) => {
                return (
                  <span
                    className="condition-chip"
                    key={id}
                    title={option ? name : `${name} — unavailable`}
                  >
                    {option && <OptionArt option={option} />}
                    <span className="condition-name">{name}</span>
                  </span>
                );
              })}
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
      {quantity != null && (
        <div data-condition="quantity">
          <dt>Quantity</dt>
          <dd>{quantity.toLocaleString()}</dd>
        </div>
      )}
    </dl>
  );
}
