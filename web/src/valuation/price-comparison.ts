/** Compare decimal amounts without losing precision through JavaScript numbers. */
export function priceComparison(price: string, estimate: string) {
  if (
    ![price, estimate].every(
      (value) => value.length <= 100 && /^\d+(\.\d+)?$/.test(value),
    )
  )
    return;
  const places = Math.max(
    ...[price, estimate].map((value) => value.split(".")[1]?.length || 0),
  );
  const units = (value: string) => {
    const [whole, fraction = ""] = value.split(".");
    return BigInt(whole + fraction.padEnd(places, "0"));
  };
  const actual = units(price),
    expected = units(estimate);
  if (expected <= 0n || actual <= 0n) return;
  const difference = actual - expected;
  if (!difference) return;
  const magnitude = difference < 0n ? -difference : difference;
  const tenths = (magnitude * 1000n + expected / 2n) / expected;
  if (!tenths) return;
  const percentage = `${tenths / 10n}${tenths % 10n ? `.${tenths % 10n}` : ""}`;
  return {
    label: `${difference < 0n ? "−" : "+"}${percentage}%`,
    description: `Ask price is about ${percentage}% ${difference < 0n ? "below" : "above"} this estimate`,
  };
}
