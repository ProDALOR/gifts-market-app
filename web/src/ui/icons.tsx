import mrktLogo from "../icons/mrkt.svg?no-inline";
export function CurrencySymbol({ currency }: { currency: string }) {
  if (currency !== "GRAM" && currency !== "XTR")
    return <small>{currency}</small>;
  return (
    <svg
      className={`currency-symbol ${currency === "XTR" ? "star" : "gram"}`}
      viewBox="0 0 24 24"
      role="img"
      aria-label={currency === "XTR" ? "Stars" : "GRAM"}
    >
      {currency === "XTR" ? (
        <path
          fill="currentColor"
          d="m12 2.5 2.93 5.94 6.56.95-4.75 4.63 1.12 6.53L12 17.47l-5.86 3.08 1.12-6.53L2.51 9.39l6.56-.95Z"
        />
      ) : (
        <path
          d="M5 4h14a2 2 0 0 1 1.7 3L12 21 3.3 7A2 2 0 0 1 5 4Zm7 0v17"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.7"
          strokeLinejoin="round"
        />
      )}
    </svg>
  );
}

export function SearchIcon() {
  return (
    <svg className="search-icon" viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="10" cy="10" r="6.5" />
      <path d="m15 15 6 6" />
    </svg>
  );
}
export function VenueIcon({ name }: { name: string }) {
  const venue = name.toLowerCase();
  if (venue.includes("mrkt"))
    return (
      <img className="venue-logo" src={mrktLogo} alt="" aria-hidden="true" />
    );
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      {venue.includes("telegram") ? (
        <path fill="currentColor" d="M2 11 21 3l-4 18-6-5-3 3v-6l9-7-11 6z" />
      ) : venue.includes("portal") ? (
        <path
          fill="none"
          stroke="currentColor"
          strokeWidth="3"
          strokeLinejoin="round"
          d="M4 20V8l10-5 6 3v12l-6 3V9L4 14"
        />
      ) : venue === "all venues" ? (
        <g fill="currentColor">
          <rect x="3" y="3" width="8" height="8" rx="2" />
          <rect x="13" y="3" width="8" height="8" rx="2" />
          <rect x="3" y="13" width="8" height="8" rx="2" />
          <rect x="13" y="13" width="8" height="8" rx="2" />
        </g>
      ) : (
        <text
          x="12"
          y="17"
          textAnchor="middle"
          fill="currentColor"
          fontSize="16"
          fontWeight="700"
        >
          {name[0]?.toUpperCase()}
        </text>
      )}
    </svg>
  );
}
