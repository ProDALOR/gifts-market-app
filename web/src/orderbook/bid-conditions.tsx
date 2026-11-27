import type { CatalogOption } from "../market";
import { giftNumberRange } from "../orderbook";
import { resolveTraits } from "../catalog/traits";
import type { BidCriteria } from "../orderbook";
import { OptionArt } from "../previews/option-art";
import { useCollectionFloors, FloorBadge } from "../valuation/floors";
import { traitFloor } from "../valuation/floors-controller";
import { UnrestrictedTrait } from "../previews/unrestricted-trait";
import { RarityLabel } from "../ui/rarity-label";
import { MarketLink } from "../app/navigation";
import { marketScopeSelection } from "./attribute-market";

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
  collection: string;
  currency: string;
}) {
  const shared = useCollectionFloors(collection, currency);
  const book = { collection_id: collection, currency_id: currency };
  const number = giftNumberRange(criteria.number);
  return (
    <dl className="bid-conditions" aria-label="Bid conditions">
      {resolveTraits(criteria, options, names).map((trait) => {
        return (
          <div key={trait.id} data-condition={trait.kind}>
            <dt>{trait.label}</dt>
            <dd>
              {!trait.entries.length && (
                <MarketLink
                  className="condition-chip condition-any"
                  selection={marketScopeSelection(book, {}, "asks")}
                  aria-label={`View asks with any ${trait.label.toLowerCase()}`}
                  title={`View asks with any ${trait.label.toLowerCase()}`}
                >
                  <UnrestrictedTrait kind={trait.kind} />
                </MarketLink>
              )}
              {trait.entries.map(({ id, name, option }) => {
                const content = (
                  <>
                    {option && <OptionArt option={option} />}
                    <span className="condition-name">
                      <span>{name}</span>
                      <RarityLabel rarity={option?.rarity} />
                    </span>
                    <FloorBadge
                      compact
                      rounded
                      floor={traitFloor(shared, trait.kind, id)}
                      currency={currency}
                      stale={shared?.stale}
                    />
                  </>
                );
                return option ? (
                  <MarketLink
                    key={id}
                    className="condition-chip condition-floor"
                    selection={marketScopeSelection(
                      book,
                      { [trait.id]: [id] },
                      "asks",
                    )}
                    aria-label={`View ${trait.label.toLowerCase()} ${name} asks`}
                    title={`View ${name} asks`}
                  >
                    {content}
                  </MarketLink>
                ) : (
                  <span
                    key={id}
                    className="condition-chip condition-floor"
                    title={`${name} — unavailable`}
                  >
                    {content}
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
            <MarketLink
              className="condition-chip condition-range"
              selection={marketScopeSelection(
                book,
                { number: criteria.number },
                "asks",
              )}
              aria-label={`View asks for gift # ${number}`}
              title={`View asks for gift # ${number}`}
            >
              #{number}
            </MarketLink>
          </dd>
        </div>
      )}
    </dl>
  );
}
