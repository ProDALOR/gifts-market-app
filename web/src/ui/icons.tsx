import mrktLogo from "../icons/mrkt.svg?no-inline";
import registry from "../../../crates/market-media/assets/icons.json";
import { useArtworkImage } from "../previews/use-artwork-image";
import { usePreviewEnvironment } from "../previews/preview-environment";

const manifest = registry.icons;
export type IconName = keyof typeof manifest;
/** Static SVG masks share the media cache and image recovery; no animation player. */
export function SharedIcon({
  name,
  className = "",
  label,
}: {
  name: IconName;
  className?: string;
  label?: string;
}) {
  return (
    <StaticIcon key={name} name={name} className={className} label={label} />
  );
}
function StaticIcon({
  name,
  className,
  label,
}: {
  name: IconName;
  className: string;
  label?: string;
}) {
  const url = `/media/icons/${name}/preview?v=${registry.preview_version}-${manifest[name].document_id}`;
  const { pageVisible } = usePreviewEnvironment();
  const image = useArtworkImage(url, undefined, pageVisible);
  return (
    <span
      className={`shared-icon ${className}`}
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    >
      {!image.ready && (
        <span
          className="shared-icon-fallback"
          data-fallback={manifest[name].alternative}
        />
      )}
      {image.sources.map((source) => (
        <img
          key={source.attempt}
          src={source.url}
          alt=""
          crossOrigin="anonymous"
          decoding="async"
          loading="lazy"
          onLoad={(event) => source.loaded(event.currentTarget)}
          onError={source.failed}
        />
      ))}
      {image.ready && (
        <span
          className="shared-icon-mask"
          style={{ maskImage: `url("${url}")` }}
        />
      )}
    </span>
  );
}
export function CurrencySymbol({ currency }: { currency: string }) {
  if (currency !== "GRAM" && currency !== "XTR")
    return <small>{currency}</small>;
  return (
    <SharedIcon
      name={currency === "XTR" ? "stars" : "gram"}
      className={`currency-symbol ${currency === "XTR" ? "star" : "gram"}`}
      label={currency === "XTR" ? "Stars" : "GRAM"}
    />
  );
}

export function SearchIcon() {
  return <SharedIcon name="search" className="search-icon" />;
}
export function VenueIcon({ name }: { name: string }) {
  const venue = name.toLowerCase();
  if (venue.includes("mrkt"))
    return (
      <img className="venue-logo" src={mrktLogo} alt="" aria-hidden="true" />
    );
  // Fragment's header glyph: https://fragment.com/css/auction.css
  if (venue.includes("fragment"))
    return (
      <svg viewBox="112 110 292 292" aria-hidden="true">
        <path
          fill="currentColor"
          fillRule="evenodd"
          d="M126.3 153.6 252 209.2c3.2 1.5 7.2 1.3 12.2-1l123.8-54.7c6.6-2.4 2.9-9.8-2.3-9l-256.6-1.1c-7.2.3-9.2 7.5-2.8 10.2Zm-10.1 28.5 118.4 184c3.7 6 13.1 2.6 12.8-2.8l-.2-126.2c-.1-7.1-1.9-8.7-6.4-10.6l-118.4-52.7c-4.3-2.4-10.1 1.6-6.2 8.3Zm151.3 52.7.3 127c.1 8.5 8.6 10.1 13.5 2.9l103-159.4 2.2-3.4 1.9-2.9v-.1l10.8-16.6c3.3-5.5-1.9-11.8-9.7-8.1L272 227.1l-.6.3c-1.5.9-2.5 1.8-3 2.9-.7 1.2-.9 2.7-.9 4.5Z"
        />
      </svg>
    );
  // Symbol from https://ddejfvww7sqtk.cloudfront.net/faq-assets/imgs/full_logo_white.svg
  if (venue.includes("getgems"))
    return (
      <svg viewBox="0 0 28.036 28.036" aria-hidden="true">
        <path
          fill="currentColor"
          fillRule="evenodd"
          d="M14.018 0c0 7.742-6.276 14.018-14.018 14.018C0 6.276 6.276 0 14.018 0zm0 28.036C6.276 28.036 0 21.76 0 14.018c7.742 0 14.018 6.276 14.018 14.018zm14.018-14.018c0 7.742-6.276 14.018-14.018 14.018 0-7.742 6.276-14.018 14.018-14.018zm-1.169-12.85c0 6.452-5.23 11.682-11.681 11.682 0-6.452 5.23-11.682 11.681-11.682z"
        />
      </svg>
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
