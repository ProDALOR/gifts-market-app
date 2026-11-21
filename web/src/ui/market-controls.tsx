import { useEffect, useRef } from "react";
import { amount, compactAmount, listingAmount } from "../amount";
import { CurrencySymbol } from "./icons";
import { usePreviewEnvironment } from "../previews/preview-environment";
export function Quote({
  price,
  currency,
  format = "exact",
}: {
  price: string;
  currency: string;
  format?: "exact" | "listing" | "compact";
}) {
  const value = useRef<HTMLSpanElement>(null);
  const previous = useRef({ price, currency });
  const { pageVisible, reducedMotion } = usePreviewEnvironment();
  useEffect(() => {
    const old = previous.current;
    previous.current = { price, currency };
    const element = value.current;
    if (
      !element ||
      old.price === price ||
      old.currency !== currency ||
      !pageVisible ||
      reducedMotion ||
      !element.animate
    )
      return;
    const box = element.getBoundingClientRect();
    if (!box.width || box.bottom <= 0 || box.top >= innerHeight) return;
    const flash = element.animate(
      [
        {
          backgroundColor:
            getComputedStyle(element)
              .getPropertyValue("--quote-flash")
              .trim() || "transparent",
        },
        { backgroundColor: "transparent" },
      ],
      { duration: 350, easing: "ease-out" },
    );
    flash.id = "price-highlight";
    return () => flash.cancel();
  }, [price, currency, pageVisible, reducedMotion]);
  const display =
    format === "compact"
      ? compactAmount(price)
      : format === "listing"
        ? listingAmount(price)
        : amount(price);
  return (
    <>
      <CurrencySymbol currency={currency} />
      <span
        ref={value}
        className={`quote-amount${display.length > 9 ? " precise" : ""}`}
        title={format !== "exact" ? `${amount(price)} ${currency}` : undefined}
        aria-label={
          format !== "exact" ? `${amount(price)} ${currency}` : undefined
        }
      >
        {display}
      </span>
    </>
  );
}
