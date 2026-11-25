import { craftTier, rarityLabel, type GiftRarity } from "../catalog/rarity";

export function RarityLabel({ rarity }: { rarity?: GiftRarity | null }) {
  const label = rarityLabel(rarity);
  return label ? (
    <span
      className="attribute-rarity"
      data-craft={craftTier(rarity)}
      title={`Rarity: ${label}`}
    >
      <span className="sr">Rarity: </span>
      {label}
    </span>
  ) : null;
}
