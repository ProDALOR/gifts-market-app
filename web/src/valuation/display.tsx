import type { components } from "../gen/api";
import { amount } from "../market";
import { Quote } from "../ui/market-controls";
type Field = components["schemas"]["DisplayField"];
type Value = components["schemas"]["DisplayValue"];
function ValueResult({ value }: { value: Value }) {
  switch (value.kind) {
    case "unavailable":
      return (
        <>
          <strong className="valuation-unavailable">{value.label}</strong>
          <p>{value.message}</p>
          {value.detail && <small>{value.detail}</small>}
        </>
      );
    case "currency":
      return (
        <strong className="valuation-value">
          <Quote price={value.amount} currency={value.currency_id} />
        </strong>
      );
    case "number":
      return (
        <strong className="valuation-value">
          {amount(value.amount)}
          {value.suffix}
        </strong>
      );
    default:
      return (
        <strong className="valuation-unavailable">
          Unsupported value format
        </strong>
      );
  }
}
/** Rendering knows value kinds, never a strategy ID, metric name, precision or suffix. */
export function ValuationFields({ fields }: { fields: Field[] }) {
  if (!fields.length) return null;
  return (
    <ul className="valuation-metrics">
      {fields.map((field) => (
        <li
          key={field.id}
          data-metric={field.id}
          className={field.primary ? "valuation-primary" : undefined}
        >
          <h4>{field.label}</h4>
          <ValueResult value={field.value} />
        </li>
      ))}
    </ul>
  );
}
