import { formatDecimal } from "../amount.ts";
export { formatDecimal } from "../amount.ts";
import type { ValuationResponse, ValuationStrategy } from "../market.ts";

type Result = ValuationResponse["results"][number];
export type FieldValue =
  | { kind: "currency"; amount: string; currency_id: string }
  | { kind: "number"; amount: string; suffix: string }
  | { kind: "unavailable"; label: string; message: string; detail?: string };
export type PresentedField = {
  id: string;
  label: string;
  primary: boolean;
  value: FieldValue;
};
const record = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === "object" && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === "string";
const count = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
/** Validate cached discovery before it controls rendering; no executable templates. */
export function validateDefinitions(rows: ValuationStrategy[]) {
  if (
    !Array.isArray(rows) ||
    rows.length > 32 ||
    new Set(rows.map((row) => row?.id)).size !== rows.length
  )
    throw new Error("Invalid valuation strategy catalog");
  for (const row of rows) {
    const display = row?.display;
    if (
      !record(row) ||
      !text(row.id) ||
      !text(row.name) ||
      !text(row.revision) ||
      !row.revision ||
      row.revision.length > 64 ||
      !Array.isArray(row.targets) ||
      typeof row.requires_period !== "boolean" ||
      !record(display) ||
      !text(display.note) ||
      !Array.isArray(display.fields) ||
      display.fields.length > 32 ||
      !Array.isArray(display.evidence) ||
      display.evidence.length > 32 ||
      new Set(display.fields.map((f) => f?.id)).size !== display.fields.length
    )
      throw new Error("Invalid valuation presentation definition");
    for (const field of display.fields) {
      if (
        !record(field) ||
        !text(field.id) ||
        !/^[a-z][a-z0-9_]*$/.test(field.id) ||
        !text(field.label) ||
        typeof field.primary !== "boolean" ||
        !record(field.format) ||
        !["currency", "number"].includes(field.format.kind) ||
        (field.format.kind === "number" && !text(field.format.suffix)) ||
        (field.format.decimals != null &&
          (!count(field.format.decimals) || field.format.decimals > 28))
      )
        throw new Error("Invalid valuation field definition");
    }
    for (const field of display.evidence) {
      if (
        !record(field) ||
        !text(field.path) ||
        !field.path.startsWith("/") ||
        /~(?![01])/.test(field.path) ||
        !text(field.label)
      )
        throw new Error("Invalid valuation evidence definition");
    }
  }
}
function pointer(data: unknown, path: string): unknown {
  let value = data;
  for (const token of path.slice(1).split("/")) {
    const key = token.replace(/~1/g, "/").replace(/~0/g, "~");
    if (Array.isArray(value)) {
      if (!/^(0|[1-9]\d*)$/.test(key) || !Number.isSafeInteger(Number(key)))
        return undefined;
      value = value[Number(key)];
    } else {
      if (!record(value) || !Object.hasOwn(value, key)) return undefined;
      value = value[key];
    }
  }
  return value;
}
const labels: Record<string, string> = {
  not_enough_data: "Not enough data",
  missing_trait: "Missing trait",
  invalid_baseline: "Invalid baseline",
  calculation_overflow: "Calculation overflow",
};
function unavailable(error: unknown): FieldValue {
  if (!record(error) || !text(error.code) || !text(error.message))
    throw new Error("Invalid valuation issue");
  if (
    (error.required_samples != null && !count(error.required_samples)) ||
    (error.available_samples != null && !count(error.available_samples))
  )
    throw new Error("Invalid valuation evidence");
  return {
    kind: "unavailable",
    label: Object.hasOwn(labels, error.code)
      ? labels[error.code]
      : "Unavailable",
    message: error.message,
    ...(count(error.required_samples)
      ? {
          detail: `${error.available_samples ?? 0} / ${error.required_samples} required samples`,
        }
      : {}),
  };
}
/** Read typed metrics once. Values and evidence remain authoritative in the wire result. */
export function presentResult(
  strategy: ValuationStrategy,
  result: Result,
  currency: string,
) {
  if (strategy.id !== result.strategy || strategy.revision !== result.revision)
    throw new Error("Valuation strategy definition changed");
  const error = "error" in result ? result.error : undefined;
  const data: unknown = "data" in result ? result.data : undefined;
  const definitions = error
    ? [
        strategy.display.fields.find((f) => f.primary) ?? {
          id: "value",
          label: "Value",
          primary: true,
          format: { kind: "currency" as const },
        },
      ]
    : strategy.display.fields;
  const fields: PresentedField[] = definitions.map((field) => {
    if (error) return { ...field, value: unavailable(error) };
    if (!record(data) || !Object.hasOwn(data, field.id))
      throw new Error("Valuation metric is missing");
    const metric = data[field.id];
    if (
      !record(metric) ||
      Object.hasOwn(metric, "value") === Object.hasOwn(metric, "error")
    )
      throw new Error("Invalid valuation metric");
    if ("error" in metric)
      return { ...field, value: unavailable(metric.error) };
    if (!text(metric.value)) throw new Error("Invalid valuation metric amount");
    const amount = formatDecimal(metric.value, field.format.decimals);
    return {
      ...field,
      value:
        field.format.kind === "currency"
          ? { kind: "currency", amount, currency_id: currency }
          : { kind: "number", amount, suffix: field.format.suffix },
    };
  });
  const evidence = error
    ? []
    : strategy.display.evidence.flatMap((field) => {
        const value = pointer(data, field.path);
        return count(value) || text(value) ? [`${value} ${field.label}`] : [];
      });
  return { fields, evidence };
}
