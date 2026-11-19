import { catalog } from "../market";
import type { components } from "../gen/api";
import { ConsoleConnection, ConsoleLog } from "./socket-console";
import type { ConnectionState, LogEntry } from "./socket-console";
import "./docs.css";

type Command = components["schemas"]["Command"];
const get = <T extends HTMLElement>(id: string) =>
  document.getElementById(id) as T;
const connect = get<HTMLButtonElement>("connect"),
  disconnect = get<HTMLButtonElement>("disconnect");
const subscribe = get<HTMLButtonElement>("subscribe"),
  cancel = get<HTMLButtonElement>("cancel");
const send = get<HTMLButtonElement>("send"),
  editor = get<HTMLTextAreaElement>("command");
const collection = get<HTMLSelectElement>("collection"),
  currency = get<HTMLSelectElement>("currency"),
  side = get<HTMLSelectElement>("side");
const id = get<HTMLInputElement>("subscription-id"),
  form = get<HTMLFormElement>("subscription");
const error = get<HTMLParagraphElement>("console-error"),
  status = get<HTMLSpanElement>("connection-state");
const retry = get<HTMLButtonElement>("retry-catalog"),
  messages = get<HTMLOListElement>("messages");
const stop = new AbortController();
const log = new ConsoleLog();
let state: ConnectionState = "closed";
let template: Extract<Command, { type: "orders.watch" }> | undefined;
let loading: AbortController | undefined;
const url = new URL("/ws", location.href);
url.protocol = location.protocol === "https:" ? "wss:" : "ws:";
get("endpoint").textContent = url.href;
const fail = (value: string) => {
  error.textContent = value;
  error.hidden = !value;
};
function controls() {
  connect.disabled = state !== "closed";
  disconnect.disabled = state === "closed";
  send.disabled = state !== "open";
  cancel.disabled = state !== "open";
  subscribe.disabled =
    state !== "open" || !template || !collection.value || !currency.value;
}
function write(direction: LogEntry["direction"], text: string) {
  const atBottom =
    messages.scrollTop + messages.clientHeight >= messages.scrollHeight - 25;
  let label = "",
    display = text;
  if (text.length <= 16_384 && direction !== "event") {
    try {
      const message = JSON.parse(text);
      if (typeof message?.type === "string")
        label = ` · ${message.type.slice(0, 64)}`;
      if (typeof message?.id === "string" && message.id)
        label += ` · ${message.id.slice(0, 64)}`;
      display = JSON.stringify(message, null, 2);
    } catch {
      /* Keep malformed server JSON visible as text. */
    }
  }
  const entry = log.push(direction, display);
  const item = document.createElement("li"),
    details = document.createElement("details"),
    summary = document.createElement("summary"),
    body = document.createElement("pre");
  body.textContent = entry.text;
  summary.textContent = `${entry.time} · ${direction}${label}${entry.clipped ? " · clipped" : ""}`;
  details.append(summary, body);
  item.append(details);
  messages.append(item);
  while (messages.children.length > log.entries.length)
    messages.firstElementChild?.remove();
  get("message-count").textContent =
    `${log.count} (${log.entries.length} retained)`;
  if (atBottom) messages.scrollTop = messages.scrollHeight;
}
const connection = new ConsoleConnection(
  url.href,
  (next, detail) => {
    state = next;
    status.dataset.state = next;
    status.textContent =
      next === "open"
        ? "Connected"
        : next === "connecting"
          ? "Connecting…"
          : "Disconnected";
    if (detail) {
      fail(detail);
      write("event", detail);
    }
    controls();
  },
  (text) => write("received", text),
);
function command(): Command {
  if (!template || !collection.value || !currency.value)
    throw new Error("Load and choose a collection and currency first.");
  return {
    ...template,
    id: id.value,
    params: {
      ...template.params,
      source: side.value === "bid" ? "bids" : "asks",
      filters: {
        ...template.params.filters,
        collection_id: collection.value,
        currency_id: currency.value,
      },
    },
  };
}
function updateEditor() {
  if (template && collection.value && currency.value)
    editor.value = JSON.stringify(command(), null, 2);
  controls();
}
function transmit(text: string) {
  try {
    connection.send(text);
    fail("");
    write("sent", text);
  } catch (cause) {
    fail(cause instanceof Error ? cause.message : String(cause));
  }
}
connect.addEventListener(
  "click",
  () => {
    fail("");
    connection.connect();
  },
  { signal: stop.signal },
);
disconnect.addEventListener("click", () => connection.disconnect(), {
  signal: stop.signal,
});
form.addEventListener(
  "submit",
  (event) => {
    event.preventDefault();
    updateEditor();
    transmit(editor.value);
  },
  { signal: stop.signal },
);
cancel.addEventListener(
  "click",
  () => {
    if (!id.reportValidity()) return;
    const message: Command = { type: "cancel", id: id.value };
    editor.value = JSON.stringify(message, null, 2);
    transmit(editor.value);
  },
  { signal: stop.signal },
);
send.addEventListener("click", () => transmit(editor.value), {
  signal: stop.signal,
});
form.addEventListener("change", updateEditor, { signal: stop.signal });
get("clear").addEventListener(
  "click",
  () => {
    log.clear();
    messages.replaceChildren();
    get("message-count").textContent = "0";
  },
  { signal: stop.signal },
);
async function loadCatalog() {
  loading?.abort();
  const attempt = (loading = new AbortController());
  const signal = AbortSignal.any([stop.signal, attempt.signal]);
  retry.hidden = true;
  try {
    const [data, response] = await Promise.all([
      catalog(signal),
      fetch("/asyncapi.json", { signal }),
    ]);
    if (!response.ok)
      throw new Error("The WebSocket specification is unavailable.");
    const spec = await response.json();
    signal.throwIfAborted();
    template = spec.components.messages["orders.watch"].examples[0].payload;
    const fill = (
      select: HTMLSelectElement,
      rows: { id: string; name: string }[],
    ) => {
      const previous = select.value;
      select.replaceChildren(
        ...rows.map((row) => new Option(row.name, row.id)),
      );
      if (rows.some((row) => row.id === previous)) select.value = previous;
    };
    fill(collection, data.collections);
    fill(currency, data.currencies);
    if (!data.collections.length || !data.currencies.length)
      fail(
        "No collections or currencies are available yet. You can still send a custom JSON command.",
      );
    else fail("");
    updateEditor();
  } catch (cause) {
    if (signal.aborted) return;
    fail(
      `${cause instanceof Error ? cause.message : String(cause)} You can reload collections or send custom JSON.`,
    );
    retry.hidden = false;
  }
}
retry.addEventListener("click", () => void loadCatalog(), {
  signal: stop.signal,
});
const reference = get<HTMLDetailsElement>("protocol-reference");
let mounted = false,
  mounting = false;
reference.addEventListener(
  "toggle",
  async () => {
    if (!reference.open || mounted || mounting || stop.signal.aborted) return;
    mounting = true;
    const host = get("asyncapi-reference");
    host.textContent = "Loading the AsyncAPI reference…";
    try {
      const { mountReference } = await import("./asyncapi");
      stop.signal.throwIfAborted();
      await mountReference(host, stop.signal);
      mounted = true;
    } catch (cause) {
      if (!stop.signal.aborted)
        host.textContent = `${cause instanceof Error ? cause.message : String(cause)} Close and reopen this section to retry.`;
    } finally {
      mounting = false;
    }
  },
  { signal: stop.signal },
);
window.addEventListener("pagehide", (event) => {
  connection.disconnect();
  if (!event.persisted) stop.abort();
});
void loadCatalog();
