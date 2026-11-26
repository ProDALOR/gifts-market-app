import { SelectPicker } from "./select-picker";
import { CurrencySymbol } from "./icons";
export function CurrencyPicker({
  currencies,
  value,
  change,
}: {
  currencies: readonly string[];
  value: string;
  change: (currency: string) => void;
}) {
  return (
    <SelectPicker
      label="Currency"
      className="currency-picker"
      options={currencies.map((value) => ({
        value,
        label: value === "XTR" ? "Stars (XTR)" : value,
      }))}
      value={value}
      change={change}
      render={(currency) => <CurrencySymbol currency={currency} />}
    />
  );
}
export const marketSortOptions = [
  { value: "time", label: "Time" },
  { value: "number", label: "Gift #" },
  { value: "price", label: "Price" },
] as const;
export function SortPicker<T extends string>({
  value,
  change,
  options,
}: {
  value: T;
  change: (value: T) => void;
  options: readonly { value: T; label: string }[];
}) {
  return (
    <SelectPicker
      label="Sort by"
      className="sort-picker"
      options={options}
      value={value}
      change={change}
    />
  );
}
