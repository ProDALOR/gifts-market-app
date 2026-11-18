import { useEffect, useRef } from "react";
import { amount } from "../market";
import { CurrencySymbol } from "./icons";
import { usePreviewEnvironment } from "../previews/preview-environment";
export function Quote({
  price,
  currency,
}: {
  price: string;
  currency: string;
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
  return (
    <>
      <CurrencySymbol currency={currency} />
      <span
        ref={value}
        className={`quote-amount${price.length > 9 ? " precise" : ""}`}
      >
        {amount(price)}
      </span>
    </>
  );
}
