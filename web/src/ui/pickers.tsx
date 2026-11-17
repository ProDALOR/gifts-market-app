import { SelectPicker } from "./select-picker";
import { CurrencySymbol } from "./icons";
export function CurrencyPicker({
  currencies,
  value,
  change,
}: {
  currencies: string[];
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
export function SortPicker({
  value,
  change,
}: {
  value: "time" | "number" | "price";
  change: (value: "time" | "number" | "price") => void;
}) {
  return (
    <SelectPicker
      label="Sort by"
      className="sort-picker"
      options={[
        { value: "time", label: "Time" },
        { value: "number", label: "Gift #" },
        { value: "price", label: "Price" },
      ]}
      value={value}
      change={change}
    />
  );
}
