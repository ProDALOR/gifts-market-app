import type { Order, CatalogOption, ValuationView } from "../market";
import { giftDetails } from "../orderbook";
import { GiftArtwork } from "../previews/gift-artwork";
import { ValuationFields } from "../valuation/display";
import { backdropPalette } from "../previews/gift-appearance";

export function OrderAttributes({
  order,
  options,
  names,
  fields = [],
  summary,
}: {
  order: Order;
  options: ReadonlyMap<string, CatalogOption>;
  names: ReadonlyMap<string, string>;
  fields?: ValuationView["display"]["fields"];
  summary?: string | null;
}) {
  const rows = giftDetails(order, names, options);
  const attributes = new Set(rows.map((row) => row.attribute));
  const general = fields.filter(
    (field) => !field.attribute || !attributes.has(field.attribute),
  );
  return (
    <section
      className="gift-attributes"
      aria-label="Gift attributes and valuation"
    >
      {rows.length > 0 && (
        <dl className="detail-attributes">
          {rows.map(({ label, value, attribute, option: visual }) => {
            return (
              <div key={label} data-attribute={attribute}>
                <dt>
                  {visual && (
                    <span
                      className={`attribute-visual attribute-${visual.kind}`}
                      aria-hidden="true"
                    >
                      <GiftArtwork
                        media={
                          visual.kind !== "backdrop"
                            ? visual.media || undefined
                            : undefined
                        }
                        palette={backdropPalette(visual)}
                        fallback={
                          visual.kind === "backdrop" && visual.palette
                            ? ""
                            : value[0] || "◇"
                        }
                        playback="static"
                        placeholderKind={
                          visual.kind === "pattern" ? "symbol" : "gift"
                        }
                      />
                    </span>
                  )}
                  <span>{label}</span>
                </dt>
                <dd>
                  <strong className="attribute-name">{value}</strong>
                  <ValuationFields
                    fields={
                      attribute
                        ? fields.filter(
                            (field) => field.attribute === attribute,
                          )
                        : []
                    }
                  />
                </dd>
              </div>
            );
          })}
        </dl>
      )}
      <div className="valuation-context">
        <ValuationFields fields={general} />
      </div>
      {summary && <p className="valuation-note">{summary}</p>}
    </section>
  );
}
