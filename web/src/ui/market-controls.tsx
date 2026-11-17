import { amount } from "../market";
import { CurrencySymbol } from "./icons";
export function Quote({
  price,
  currency,
}: {
  price: string;
  currency: string;
}) {
  return (
    <>
      <CurrencySymbol currency={currency} />
      <span className={`quote-amount${price.length > 9 ? " precise" : ""}`}>
        {amount(price)}
      </span>
    </>
  );
}
