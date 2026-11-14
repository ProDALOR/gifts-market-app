import React from "react";
import type { Entry, Predicate, AttrValue } from "./data";
import { dateInput, timestamp } from "./data";
type Attribute = Extract<Entry, { kind: "attribute" }>;
type Rule = { required: boolean; allowed_values?: string[] | null };
export function Field({
  id,
  rule,
  attributes,
  options,
  value,
  change,
  depth = 0,
}: {
  id: string;
  rule: Rule;
  attributes: Map<string, Attribute>;
  options: Map<string, Extract<Entry, { kind: "attribute_option" }>>;
  value?: Predicate;
  change: (v: Predicate | undefined) => void;
  depth?: number;
}) {
  const attr = attributes.get(id);
  if (!attr || depth > 8) return null;
  const label = attr.name;
  const eq = value?.op === "eq" ? value.value : undefined;
  if (attr.value_type === "enum")
    return (
      <label>
        {label}
        <select
          aria-label={label}
          value={eq?.type === "enum" ? eq.value : ""}
          onChange={(e) =>
            change(
              e.target.value
                ? { op: "eq", value: { type: "enum", value: e.target.value } }
                : undefined,
            )
          }
        >
          <option value="">Any {label.toLowerCase()}</option>
          {(
            rule.allowed_values ||
            [...options.values()]
              .filter((v) => v.attribute_id === id)
              .map((v) => v.id)
          ).map((id) => (
            <option key={id} value={id}>
              {options.get(id)?.name || id}
            </option>
          ))}
        </select>
      </label>
    );
  if (attr.value_type === "object")
    return (
      <fieldset>
        <legend>{label}</legend>
        {Object.entries(attr.fields || {}).map(([key, child]) => (
          <Field
            key={key}
            id={key}
            rule={child}
            attributes={attributes}
            options={options}
            depth={depth + 1}
            value={value?.op === "object" ? value.fields?.[key] : undefined}
            change={(v) => {
              const fields = {
                ...(value?.op === "object" ? value.fields : {}),
              };
              if (v) fields[key] = v;
              else delete fields[key];
              change(
                Object.keys(fields).length
                  ? { op: "object", fields }
                  : undefined,
              );
            }}
          />
        ))}
      </fieldset>
    );
  if (attr.value_type === "list" && attr.element)
    return (
      <fieldset>
        <legend>{label}</legend>
        <Field
          id={attr.element}
          rule={{ required: false }}
          attributes={attributes}
          options={options}
          depth={depth + 1}
          value={value?.op === "contains" ? value.predicate : undefined}
          change={(predicate) =>
            change(
              predicate
                ? {
                    op: "contains",
                    predicate,
                    min_count: value?.op === "contains" ? value.min_count : 1,
                    max_count:
                      value?.op === "contains" ? value.max_count : null,
                  }
                : undefined,
            )
          }
        />
        {value?.op === "contains" && (
          <div className="range">
            <label>
              At least
              <input
                aria-label={`${label} minimum count`}
                type="number"
                min="0"
                value={value.min_count ?? 1}
                onChange={(e) =>
                  change({
                    ...value,
                    min_count:
                      e.target.value === "" ? null : Number(e.target.value),
                  })
                }
              />
            </label>
            <label>
              At most
              <input
                aria-label={`${label} maximum count`}
                type="number"
                min="0"
                value={value.max_count ?? ""}
                onChange={(e) =>
                  change({
                    ...value,
                    max_count:
                      e.target.value === "" ? null : Number(e.target.value),
                  })
                }
              />
            </label>
          </div>
        )}
      </fieldset>
    );
  if (attr.value_type === "integer" || attr.value_type === "decimal")
    return (
      <label>
        {label}
        <span className="range">
          {(["min", "max"] as const).map((bound) => (
            <input
              key={bound}
              aria-label={`${label} ${bound}`}
              inputMode="decimal"
              placeholder={bound === "min" ? "Min" : "Max"}
              value={
                value?.op === "range" ? String(value[bound]?.value ?? "") : ""
              }
              onChange={(e) => {
                const prior =
                  value?.op === "range"
                    ? value
                    : { op: "range" as const, min: null, max: null };
                const next = {
                  ...prior,
                  [bound]: e.target.value
                    ? ({
                        type: attr.value_type,
                        value: e.target.value,
                      } as AttrValue)
                    : null,
                };
                change(next.min || next.max ? next : undefined);
              }}
            />
          ))}
        </span>
      </label>
    );
  if (attr.value_type === "boolean")
    return (
      <label>
        {label}
        <select
          value={eq?.type === "boolean" ? String(eq.value) : ""}
          onChange={(e) =>
            change(
              e.target.value
                ? {
                    op: "eq",
                    value: {
                      type: "boolean",
                      value: e.target.value === "true",
                    },
                  }
                : undefined,
            )
          }
        >
          <option value="">Any</option>
          <option value="true">Yes</option>
          <option value="false">No</option>
        </select>
      </label>
    );
  return (
    <label>
      {label}
      <input
        type={attr.value_type === "timestamp" ? "datetime-local" : "text"}
        step={attr.value_type === "timestamp" ? "1" : undefined}
        value={
          eq?.type === "timestamp"
            ? dateInput(eq.value)
            : eq?.type === "text"
              ? eq.value
              : ""
        }
        placeholder={
          attr.value_type === "timestamp"
            ? "YYYY-MM-DDTHH:mm:ssZ"
            : `Any ${label.toLowerCase()}`
        }
        onChange={(e) =>
          change(
            e.target.value
              ? {
                  op: "eq",
                  value: {
                    type:
                      attr.value_type === "timestamp" ? "timestamp" : "text",
                    value:
                      attr.value_type === "timestamp"
                        ? timestamp(e.target.value)
                        : e.target.value,
                  },
                }
              : undefined,
          )
        }
      />
    </label>
  );
}
