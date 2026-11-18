import type { Order, CatalogOption } from "../market";
import { giftDetails } from "../orderbook";
import { GiftArtwork } from "../previews/gift-artwork";
import { backdropPalette } from "../previews/gift-appearance";

export function OrderAttributes({
  order,
  options,
  names,
}: {
  order: Order;
  options: ReadonlyMap<string, CatalogOption>;
  names: ReadonlyMap<string, string>;
}) {
  const rows = giftDetails(order, names, options);
  if (!rows.length) return null;
  return (
    <dl className="detail-attributes">
      {rows.map(({ label, value, option: visual }) => {
        return (
          <div key={label}>
            <dt>{label}</dt>
            <dd>
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
                  />
                </span>
              )}
              <span>{value}</span>
            </dd>
          </div>
        );
      })}
    </dl>
  );
}
