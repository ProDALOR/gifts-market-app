import { useId } from "react";

// Shared local shapes: no per-model outline download or renderer is needed.
const silhouettes = {
  gift: "M88 200Q88 184 104 184H408Q424 184 424 200V272H88ZM112 296H400V448Q400 464 384 464H128Q112 464 112 448ZM256 184C192 184 136 160 136 112C136 64 224 64 256 144C288 64 376 64 376 112C376 160 320 184 256 184Z",
  symbol: "M256 64L308 204L448 256L308 308L256 448L204 308L64 256L204 204Z",
};

export function GiftPlaceholder({
  kind = "gift",
  active,
  loading,
}: {
  kind?: "gift" | "symbol";
  active: boolean;
  loading: boolean;
}) {
  const id = `gift-placeholder-${useId()}`;
  return (
    <svg
      className={`gift-loading-placeholder gift-placeholder-${kind}`}
      viewBox="0 0 512 512"
      aria-hidden="true"
    >
      <defs>
        <clipPath id={`${id}-clip`}>
          <path d={silhouettes[kind]} />
        </clipPath>
        <linearGradient id={`${id}-shine`}>
          <stop offset="20%" stopColor="currentColor" stopOpacity="0" />
          <stop offset="50%" stopColor="currentColor" stopOpacity="0.25" />
          <stop offset="80%" stopColor="currentColor" stopOpacity="0" />
        </linearGradient>
      </defs>
      <g clipPath={`url(#${id}-clip)`}>
        <rect width="512" height="512" fill="currentColor" fillOpacity="0.12" />
        {active && loading && (
          <rect
            className="gift-placeholder-shimmer"
            width="512"
            height="512"
            fill={`url(#${id}-shine)`}
          />
        )}
      </g>
    </svg>
  );
}
