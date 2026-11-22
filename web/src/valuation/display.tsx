import type {
  PresentedField as Field,
  FieldValue as Value,
} from "./presentation";
import { amount } from "../market";
import { trimDecimalZeros } from "../amount";
import { Quote } from "../ui/market-controls";
function ValueResult({ value, compact }: { value: Value; compact: boolean }) {
  switch (value.kind) {
    case "unavailable":
      return (
        <>
          <strong className="valuation-unavailable">{value.label}</strong>
          {!compact && <p>{value.message}</p>}
          {!compact && value.detail && <small>{value.detail}</small>}
        </>
      );
    case "currency":
      return (
        <strong className="valuation-value">
          <Quote
            price={trimDecimalZeros(value.amount)}
            currency={value.currency_id}
          />
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
export function ValuationFields({
  fields,
  compact = false,
}: {
  fields: Field[];
  compact?: boolean;
}) {
  if (!fields.length) return null;
  return (
    <ul className="valuation-metrics">
      {fields.map((field) => (
        <li
          key={field.id}
          data-metric={field.id}
          className={field.primary ? "valuation-primary" : undefined}
        >
          <h4 className={compact && fields.length === 1 ? "sr" : undefined}>
            {field.label}
          </h4>
          <ValueResult value={field.value} compact={compact} />
        </li>
      ))}
    </ul>
  );
}
