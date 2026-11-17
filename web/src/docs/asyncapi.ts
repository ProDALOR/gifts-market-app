/** A small view of the server's AsyncAPI document; schemas and examples stay server-owned. */
type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
type ObjectValue = { [key: string]: Json };
const object = (value: Json | undefined): ObjectValue =>
  value && typeof value === "object" && !Array.isArray(value) ? value : {};
const array = (value: Json | undefined): Json[] =>
  Array.isArray(value) ? value : [];
const text = (value: Json | undefined) =>
  typeof value === "string" ? value : "";
function node<K extends keyof HTMLElementTagNameMap>(tag: K, content?: string) {
  const element = document.createElement(tag);
  if (content !== undefined) element.textContent = content;
  return element;
}
function reference(spec: Json, value: Json): Json {
  const pointer = text(object(value).$ref);
  return pointer.startsWith("#/")
    ? pointer
        .slice(2)
        .split("/")
        .reduce<Json>(
          (value, key) =>
            Array.isArray(value)
              ? (value[Number(key)] ?? null)
              : (object(value)[key.replace(/~1/g, "/").replace(/~0/g, "~")] ??
                null),
          spec,
        )
    : value;
}
function source(value: Json, label: string) {
  const details = node("details"),
    pre = node("pre");
  pre.textContent = JSON.stringify(value, null, 2);
  details.append(node("summary", label), pre);
  return details;
}
function type(value: Json): string {
  const schema = object(value);
  if (schema.$ref) return text(schema.$ref).split("/").at(-1) || "schema";
  if (schema.enum)
    return array(schema.enum)
      .map((value) => JSON.stringify(value))
      .join(" | ");
  if (schema.oneOf || schema.anyOf)
    return array(schema.oneOf || schema.anyOf)
      .map(type)
      .join(" | ");
  if (schema.type === "array") return `Array<${type(schema.items || {})}>`;
  return Array.isArray(schema.type)
    ? schema.type.map(String).join(" | ")
    : text(schema.type) || "object";
}
function fields(value: Json) {
  const schema = object(value),
    properties = object(schema.properties);
  if (!Object.keys(properties).length) return node("p", type(value));
  const table = node("table"),
    head = node("thead"),
    headings = node("tr"),
    body = node("tbody");
  for (const label of ["Field", "Type", "Description"])
    headings.append(node("th", label));
  head.append(headings);
  for (const [name, definition] of Object.entries(properties)) {
    const row = node("tr"),
      field = node("td"),
      schema = object(definition),
      kind = node("td");
    field.append(node("code", name));
    if (array(object(value).required).includes(name))
      field.append(node("small", "required"));
    const ref = text(schema.$ref);
    if (ref.startsWith("#/components/schemas/")) {
      const link = node("a", type(definition));
      link.href = `#ws-schema-${encodeURIComponent(ref.split("/")[3])}`;
      link.dataset.schema = "true";
      kind.append(link);
    } else kind.append(node("code", type(definition)));
    const constraints = [
      "minimum",
      "maximum",
      "minLength",
      "maxLength",
      "maxItems",
      "pattern",
      "format",
    ].flatMap((key) =>
      schema[key] === undefined
        ? []
        : [
            `${key}: ${typeof schema[key] === "string" ? schema[key] : JSON.stringify(schema[key])}`,
          ],
    );
    row.append(
      field,
      kind,
      node(
        "td",
        [text(schema.description), ...constraints].filter(Boolean).join(" · "),
      ),
    );
    body.append(row);
  }
  table.append(head, body);
  const scroll = node("div");
  scroll.className = "schema-table";
  scroll.append(table);
  return scroll;
}
export async function mountReference(
  element: HTMLElement,
  signal: AbortSignal,
) {
  const response = await fetch("/asyncapi.json", { signal });
  if (!response.ok)
    throw new Error("The WebSocket specification is unavailable.");
  const spec: Json = await response.json();
  signal.throwIfAborted();
  if (object(spec).asyncapi !== "3.0.0")
    throw new Error(
      "Unsupported AsyncAPI version. Download the JSON document to inspect it.",
    );
  const components = object(object(spec).components),
    messages = object(components.messages),
    info = object(object(spec).info);
  const fragment = document.createDocumentFragment();
  fragment.append(
    node("h2", text(info.title)),
    node("p", `AsyncAPI ${object(spec).asyncapi} · API ${info.version}`),
  );
  const navigation = node("nav");
  navigation.ariaLabel = "WebSocket messages";
  for (const name of Object.keys(messages)) {
    const link = node("a", name);
    link.href = `#ws-message-${name}`;
    navigation.append(link);
  }
  fragment.append(navigation);
  for (const [name, value] of Object.entries(messages)) {
    const message = object(value),
      article = node("article");
    article.id = `ws-message-${name}`;
    const incoming = Object.values(object(object(spec).operations)).some(
      (value) => {
        const operation = object(value);
        return (
          operation.action === "receive" &&
          array(operation.messages).some((ref) =>
            text(object(ref).$ref).endsWith(`/messages/${name}`),
          )
        );
      },
    );
    article.append(
      node("p", incoming ? "CLIENT → SERVER" : "SERVER → CLIENT"),
      node("h3", name),
      node("p", text(message.summary)),
    );
    const payload = reference(spec, message.payload || {});
    article.append(fields(payload));
    for (const example of array(message.examples))
      article.append(source(object(example).payload || {}, "JSON example"));
    fragment.append(article);
  }
  fragment.append(node("h3", "Referenced schemas"));
  for (const [name, schema] of Object.entries(object(components.schemas))) {
    const details = node("details");
    details.id = `ws-schema-${encodeURIComponent(name)}`;
    details.append(
      node("summary", name),
      fields(schema),
      source(schema, "JSON Schema"),
    );
    fragment.append(details);
  }
  element.replaceChildren(fragment);
  element.addEventListener(
    "click",
    (event) => {
      const link =
        event.target instanceof Element
          ? event.target.closest<HTMLAnchorElement>("a[data-schema]")
          : null;
      if (!link) return;
      const target = document.getElementById(link.hash.slice(1));
      if (target instanceof HTMLDetailsElement) target.open = true;
    },
    { signal },
  );
}
