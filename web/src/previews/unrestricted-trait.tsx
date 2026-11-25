import { SharedIcon, type IconName } from "../ui/icons";

const traits = {
  model: { label: "Any model", icon: "model" },
  backdrop: { label: "Any backdrop", icon: "backdrop" },
  pattern: { label: "Any symbol", icon: "symbol" },
} as const satisfies Record<string, { label: string; icon: IconName }>;

/** Shared static icons keep unrestricted choices distinct from missing catalog data. */
export function UnrestrictedTrait({
  kind,
  decorative = false,
}: {
  kind: keyof typeof traits;
  decorative?: boolean;
}) {
  const { label, icon } = traits[kind];
  return (
    <span
      className="unrestricted-trait"
      role={decorative ? undefined : "img"}
      aria-label={decorative ? undefined : label}
      aria-hidden={decorative || undefined}
      title={label}
    >
      <SharedIcon name={icon} />
      <span aria-hidden="true">∞</span>
    </span>
  );
}
