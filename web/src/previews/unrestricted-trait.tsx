import { GiftPlaceholder } from "./gift-placeholder";

const traits = {
  model: { label: "Any model", shape: "gift" },
  backdrop: { label: "Any backdrop", shape: "backdrop" },
  pattern: { label: "Any symbol", shape: "symbol" },
} as const;

/** Local static shapes keep unrestricted choices distinct from missing catalog data. */
export function UnrestrictedTrait({
  kind,
  decorative = false,
}: {
  kind: keyof typeof traits;
  decorative?: boolean;
}) {
  const { label, shape } = traits[kind];
  return (
    <span
      className="unrestricted-trait"
      role={decorative ? undefined : "img"}
      aria-label={decorative ? undefined : label}
      aria-hidden={decorative || undefined}
      title={label}
    >
      <GiftPlaceholder kind={shape} active={false} loading={false} />
      <span aria-hidden="true">∞</span>
    </span>
  );
}
