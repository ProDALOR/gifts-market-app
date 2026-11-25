const decimal = /^(?:0|[1-9]\d*)(?:\.\d+)?$/;

/** Gift numbers use space grouping independently of price formatting and locale. */
export function formatGiftNumber(value: number | string) {
  return String(value).replace(/\B(?=(\d{3})+(?!\d))/g, "\u00a0");
}

/** Exact decimal rounding. Strategy formatting defaults to Rust's midpoint-to-even. */
export function formatDecimal(
  value: string,
  decimals?: number | null,
  rounding: "half_even" | "half_up" = "half_even",
): string {
  if (!decimal.test(value) || value.length > 80)
    throw new Error("Invalid decimal amount");
  if (decimals == null) return value;
  if (!Number.isInteger(decimals) || decimals < 0 || decimals > 28)
    throw new Error("Unsupported decimal precision");
  const [whole, fraction = ""] = value.split(".");
  const scale = 10n ** BigInt(decimals);
  let units =
    BigInt(whole) * scale +
    BigInt((fraction + "0".repeat(decimals)).slice(0, decimals) || "0");
  const remainder = fraction.slice(decimals);
  if (
    remainder &&
    (remainder[0] > "5" ||
      (remainder[0] === "5" &&
        (/[1-9]/.test(remainder.slice(1)) ||
          rounding === "half_up" ||
          units % 2n !== 0n)))
  )
    units++;
  return decimals
    ? `${units / scale}.${String(units % scale).padStart(decimals, "0")}`
    : String(units);
}

/** Exact string grouping, without conversion to JavaScript floating point. */
export function amount(value: string) {
  const [whole, fraction] = value.split(".");
  return (
    whole.replace(/\B(?=(\d{3})+(?!\d))/g, "\u00a0") +
    (fraction ? "." + fraction : "")
  );
}
export const trimDecimalZeros = (value: string) =>
  value.includes(".") ? value.replace(/0+$/, "").replace(/\.$/, "") : value;
export const roundedAmount = (value: string) =>
  trimDecimalZeros(formatDecimal(value, 2, "half_up"));

/** Small collection tiles use a decimal magnitude; full prices remain available on intent. */
export function compactAmount(value: string) {
  formatDecimal(value); // Validate the exact input before shifting its decimal point.
  const [whole, fraction = ""] = value.split(".");
  const suffixes = ["", "K", "M", "B", "T"];
  let exponent =
    whole.length > 15
      ? whole.length - 1
      : Math.floor((whole.length - 1) / 3) * 3;
  const scaled = (places: number) => {
    const digits = whole.padStart(places + 1, "0");
    const point = digits.length - places;
    return (
      digits.slice(0, point) +
      "." +
      digits.slice(point) +
      fraction
    ).replace(/\.$/, "");
  };
  let rounded = roundedAmount(scaled(exponent));
  if (rounded === (exponent > 12 ? "10" : "1000")) {
    exponent += exponent > 12 ? 1 : 3;
    rounded = roundedAmount(scaled(exponent));
  }
  return rounded + (suffixes[exponent / 3] ?? `e${exponent}`);
}

/** Ellipsis means fractional digits were hidden, rather than changing the quoted price. */
export function listingAmount(value: string) {
  const [whole, fraction] = value.split(".");
  return (
    amount(whole + (fraction ? "." + fraction.slice(0, 2) : "")) +
    (fraction && fraction.length > 2 ? ".." : "")
  );
}

/** Sum quoted amounts without passing through binary floating point. */
export function sumAmounts(values: Iterable<string>): string {
  const scale = 10n ** 28n;
  let total = 0n;
  for (const value of values) {
    formatDecimal(value);
    const [whole, fraction = ""] = value.split(".");
    if (fraction.length > 28) throw new Error("Unsupported decimal precision");
    total += BigInt(whole) * scale + BigInt(fraction.padEnd(28, "0"));
  }
  return trimDecimalZeros(
    `${total / scale}.${String(total % scale).padStart(28, "0")}`,
  );
}
