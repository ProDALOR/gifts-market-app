import assert from "node:assert/strict";
import { test } from "node:test";
import {
  formatDecimal,
  presentResult,
  validateDefinitions,
} from "../src/valuation/presentation.ts";
import type { ValuationResponse, ValuationStrategy } from "../src/market.ts";
type Result = ValuationResponse["results"][number];
const strategy: ValuationStrategy = {
  purpose: "estimate",
  id: "future",
  name: "Future strategy",
  description: "Future calculation",
  input: { targets: ["gift"], period: "unsupported" },
  revision: "r1",
  documentation_url: "/guide",
  documentation_revision: "d1",
  display: {
    fields: [
      {
        id: "decision",
        label: "Decision",
        primary: true,
        format: { kind: "currency", decimals: 2 },
      },
      {
        id: "score",
        label: "Confidence",
        primary: false,
        format: { kind: "number", decimals: 3, suffix: "%" },
      },
    ],
    evidence: [
      { path: "/evidence/gifts", label: "gifts" },
      { path: "/evidence/a~1b/~0count", label: "observations" },
      { path: "/missing", label: "missing" },
    ],
    note: "Strategy-owned note",
  },
};
const outcome = (data: unknown): Result =>
  ({ strategy: "future", revision: "r1", data }) as Result;

test("presentation reads typed metrics and evidence using discovered fields without mutating the report", () => {
  validateDefinitions([strategy]);
  const result = outcome({
    decision: { value: "10000000000000000.125" },
    score: { value: "87.5" },
    evidence: { gifts: 0, "a/b": { "~count": 4 } },
  });
  const before = JSON.stringify(result);
  const display = presentResult(strategy, result, "XTR");
  assert.deepEqual(
    display.fields.map((f) => f.value),
    [
      { kind: "currency", amount: "10000000000000000.12", currency_id: "XTR" },
      { kind: "number", amount: "87.500", suffix: "%" },
    ],
  );
  assert.deepEqual(display.evidence, ["0 gifts", "4 observations"]);
  assert.equal(JSON.stringify(result), before);
});

test("evidence pointers support array indices, escaped keys and never inherited properties", () => {
  const metadata = {
    ...strategy,
    display: {
      ...strategy.display,
      evidence: [
        { path: "/observations/0/gifts", label: "samples" },
        { path: "/observations/length", label: "not an array index" },
        { path: "/observations/01/gifts", label: "not a canonical index" },
        { path: "/evidence/toString", label: "inherited" },
      ],
    },
  };
  const display = presentResult(
    metadata,
    outcome({
      decision: { value: "1" },
      score: { value: "2" },
      observations: [{ gifts: 4 }],
      evidence: {},
    }),
    "GRAM",
  );
  assert.deepEqual(display.evidence, ["4 samples"]);
});

test("decimal formatting agrees with Rust midpoint-to-even including carries and exact large values", () => {
  for (const [input, precision, expected] of [
    ["20.125", 2, "20.12"],
    ["20.135", 2, "20.14"],
    ["2.5", 0, "2"],
    ["3.5", 0, "4"],
    ["1.2", 3, "1.200"],
    ["9.995", 2, "10.00"],
    ["1.245000001", 2, "1.25"],
    ["0.0050000", 2, "0.00"],
    ["10000000000000000.000000001", null, "10000000000000000.000000001"],
    ["0.0000000000000000000000000001", 28, "0.0000000000000000000000000001"],
  ] as const)
    assert.equal(formatDecimal(input, precision), expected);
  for (const invalid of ["NaN", "Infinity", "-1", "1e10", "0x20", "", "1.2.3"])
    assert.throws(() => formatDecimal(invalid, 2));
  for (const precision of [-1, 29, 1.5])
    assert.throws(() => formatDecimal("1", precision));
});

test("independent metric issues retain sample evidence and failed strategies render only once", () => {
  const metric = {
    error: {
      code: "not_enough_data",
      message: "More gifts needed",
      available_samples: 1,
      required_samples: 3,
    },
  };
  const display = presentResult(
    strategy,
    outcome({ decision: metric, score: { value: "10" } }),
    "GRAM",
  );
  assert.deepEqual(display.fields[0].value, {
    kind: "unavailable",
    label: "Not enough data",
    message: "More gifts needed",
    detail: "1 / 3 required samples",
  });
  const failed = {
    strategy: "future",
    revision: "r1",
    error: { code: "capacity_exceeded", message: "Try later" },
  } as Result;
  const failure = presentResult(strategy, failed, "GRAM");
  assert.equal(failure.fields.length, 1);
  assert.deepEqual(failure.evidence, []);
  assert.deepEqual(failure.fields[0].value, {
    kind: "unavailable",
    label: "Unavailable",
    message: "Try later",
  });
});

test("stale contracts and malformed metrics fail before rendering", () => {
  const data = { decision: { value: "1" }, score: { value: "2" } };
  assert.throws(
    () => presentResult(strategy, { ...outcome(data), revision: "r2" }, "GRAM"),
    /definition changed/,
  );
  for (const decision of [
    null,
    [],
    {},
    { value: 2 },
    { value: "1", error: {} },
    { error: { code: "not_enough_data" } },
    { value: "NaN" },
  ])
    assert.throws(() =>
      presentResult(strategy, outcome({ ...data, decision }), "GRAM"),
    );
  assert.throws(
    () => presentResult(strategy, outcome({ score: { value: "2" } }), "GRAM"),
    /missing/,
  );
  const invalid = {
    ...strategy,
    display: {
      ...strategy.display,
      fields: [strategy.display.fields[0], strategy.display.fields[0]],
    },
  };
  assert.throws(() => validateDefinitions([invalid]));
  for (const patch of [
    { revision: "" },
    { display: null },
    {
      display: {
        ...strategy.display,
        fields: [{ ...strategy.display.fields[0], format: { kind: "script" } }],
      },
    },
  ])
    assert.throws(() =>
      validateDefinitions([{ ...strategy, ...patch } as ValuationStrategy]),
    );
  assert.throws(() => validateDefinitions([strategy, strategy]));
});

test("flat reference floors use a discovered root pointer with a single count", () => {
  const reference: ValuationStrategy = {
    ...strategy,
    purpose: "reference",
    display: {
      fields: [
        {
          id: "price",
          label: "Matching floor",
          primary: true,
          path: "",
          format: { kind: "currency" },
        },
      ],
      evidence: [{ path: "/count", label: "gifts" }],
      note: "Current asks",
    },
  };
  validateDefinitions([reference]);
  const rendered = presentResult(
    reference,
    outcome({ value: "10.800000001", count: 0 }),
    "GRAM",
  );
  assert.deepEqual(rendered.fields[0].value, {
    kind: "currency",
    amount: "10.800000001",
    currency_id: "GRAM",
  });
  assert.deepEqual(rendered.evidence, ["0 gifts"]);
  const missing = presentResult(
    reference,
    outcome({
      error: { code: "not_enough_data", message: "No matching asks" },
      count: 0,
    }),
    "GRAM",
  );
  assert.equal(missing.fields[0].value.kind, "unavailable");
  for (const path of ["bad", "/~2bad"])
    assert.throws(() =>
      validateDefinitions([
        {
          ...reference,
          display: {
            ...reference.display,
            fields: [{ ...reference.display.fields[0], path }],
          },
        },
      ]),
    );
  assert.throws(() =>
    presentResult(
      {
        ...reference,
        display: {
          ...reference.display,
          fields: [{ ...reference.display.fields[0], path: "/missing" }],
        },
      },
      outcome({ value: "10", count: 1 }),
      "GRAM",
    ),
  );
});
