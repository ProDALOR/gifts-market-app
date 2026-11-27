import { GiftPlaceholder } from "../previews/gift-placeholder";

/** A fixed, static first-page shell; it creates no image requests or animation players. */
export function GiftGridSkeleton() {
  return (
    <>
      <li className="sr" role="status">
        Loading gifts…
      </li>
      {Array.from({ length: 12 }, (_, index) => (
        <li
          key={index}
          className="gift-card gift-card-loading"
          aria-hidden="true"
        >
          <div className="card-preview">
            <GiftPlaceholder active={false} loading={false} />
            <span className="card-loading-copy">
              <span />
              <span />
            </span>
          </div>
        </li>
      ))}
    </>
  );
}
