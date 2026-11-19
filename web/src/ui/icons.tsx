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
  // Vector redraws of the glyphs used by https://t.me/portals and
  // https://t.me/Tonnel_Network_bot; inherit the surrounding theme color.
  if (venue.includes("portal"))
    return (
      <svg viewBox="0 0 152 184" aria-hidden="true">
        <path
          fill="currentColor"
          fillRule="evenodd"
          d="M76 0C34 0 0 34 0 76v94c0 8 5 14 14 14h10c21 0 28-20 52-20s31 20 52 20h10c9 0 14-6 14-14V76C152 34 118 0 76 0ZM76 21c30 0 55 25 55 55v85c-8-1-15-8-21-14V96c0-20-13-31-34-31S42 76 42 96v51c-6 6-13 13-21 14V76c0-30 25-55 55-55Zm-12 75c0-8 4-12 12-12s12 4 12 12v45c0 3-3 5-6 5H70c-3 0-6-2-6-5V96Z"
        />
      </svg>
    );
  if (venue.includes("tonnel"))
    return (
      <svg viewBox="67 42 186 237" aria-hidden="true">
        <g
          fill="none"
          stroke="currentColor"
          strokeWidth="6.5"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M110 62a87 87 0 0 1 137 73v137M92 80a87 87 0 0 0-19 55v137" />
          <path d="M87 188v-53a73 73 0 0 1 143-19M87 202v70M233 135v137" />
          <path d="M101 142v-7a59 59 0 0 1 91-50M101 158v9M101 181v91M207 102a59 59 0 0 1 12 33v137" />
          <path d="M127 104a45 45 0 0 1 78 31v12M115 117a45 45 0 0 0 0 18v137M205 159v8M205 181v91" />
          <path d="M129 272V135a31 31 0 0 1 62 0v137" />
        </g>
        <path
          fill="currentColor"
          d="M143 165v-29a17 17 0 0 1 34 0v29Zm13 63h8l5 18h-18Z"
        />
        <circle cx="160" cy="220" r="9" fill="currentColor" />
      </svg>
    );
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      {venue.includes("telegram") ? (
        <path fill="currentColor" d="M2 11 21 3l-4 18-6-5-3 3v-6l9-7-11 6z" />
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
