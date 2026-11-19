import type { CatalogOption } from "../market";
import { traits, giftNumberRange } from "../orderbook";
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
      {traits.map((trait) => {
        const ids = criteria[trait.id];
        return (
          <div key={trait.id} data-condition={trait.kind}>
            <dt>{trait.label}</dt>
            <dd>
              {!ids.length && (
                <span className="condition-any">
                  Any {trait.label.toLowerCase()}
                </span>
              )}
              {ids.map((id) => {
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
