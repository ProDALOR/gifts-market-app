import type { CatalogOption } from "../market";
import { giftNumberRange } from "../orderbook";
import { resolveTraits } from "../catalog/traits";
import type { BidCriteria } from "../orderbook";
import { OptionArt } from "../previews/option-art";
import { TraitFloor } from "../valuation/floors";
import { UnrestrictedTrait } from "../previews/unrestricted-trait";

/** Complete accepted alternatives for bid details. */
export function BidConditions({
  criteria,
  options,
  names,
  collection,
  currency,
}: {
  criteria: BidCriteria;
  options: ReadonlyMap<string, CatalogOption>;
  names?: ReadonlyMap<string, string>;
  collection?: string;
  currency?: string;
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
                <details className="condition-any">
                  <summary
                    className="condition-chip"
                    aria-label={`Any ${trait.label.toLowerCase()}`}
                    title={`Any ${trait.label.toLowerCase()}`}
                  >
                    <UnrestrictedTrait kind={trait.kind} decorative />
                  </summary>
                  <span className="condition-any-label">
                    Any {trait.label.toLowerCase()}
                  </span>
                </details>
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
                    <TraitFloor
                      collection={collection}
                      currency={currency}
                      id={id}
                      kind={trait.kind}
                    />
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
    </dl>
  );
}
