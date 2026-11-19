import type { components } from "./gen/api.ts";
import { retryDelay } from "./retry.ts";
import { ApiFailure } from "./api-error.ts";
import { parseFrame } from "./ws-protocol.ts";
import type {
  LiveFrame,
  RequestCall,
  RequestResult,
  SubscriptionCommand,
} from "./ws-protocol.ts";
export type { LiveFrame } from "./ws-protocol.ts";
type Call = RequestCall extends infer R
  ? R extends { id: string }
    ? Omit<R, "id">
    : never
  : never;
type Results = { [R in RequestResult as R["method"]]: R["data"] };
type Result<M extends keyof Results> = Results[M];
type Subscription = {
  kind: "subscription";
  command: SubscriptionCommand;
  resume?: SubscriptionCommand;
  wire?: string;
  frame: (value: LiveFrame) => void;
  status?: (value: string) => void;
  failures: number;
  retry?: ReturnType<typeof setTimeout>;
};
type Pending = {
  kind: "request";
  wire: string;
  call: Call;
  sent: boolean;
  finish: (error?: unknown, result?: RequestResult["data"]) => void;
};
type Operation = Subscription | Pending;
/** One bounded connection: persistent subscriptions reconnect; one-shot requests fail once. */
export class LiveSocket {
  private statusListeners = new Set<
    (status: components["schemas"]["MarketStatus"]) => void
  >();
  private availability: Readonly<{
    status: components["schemas"]["MarketStatus"];
    generation: number;
  }> = { status: "unavailable", generation: 0 };
  readonly statusSnapshot = () => this.availability;
  private publishStatus(status: components["schemas"]["MarketStatus"]) {
    if (status === this.availability.status) return;
    const generation =
      this.availability.generation +
      Number(
        this.availability.status === "available" && status !== "available",
      );
    this.availability = { status, generation };
    for (const callback of this.statusListeners) callback(status);
  }
  watchStatus(
    callback: (status: components["schemas"]["MarketStatus"]) => void,
  ) {
    this.statusListeners.add(callback);
    callback(this.availability.status);
    this.ensureConnected();
    return () => {
      this.statusListeners.delete(callback);
      this.closeIdle();
    };
  }
  private listeners = new Map<string, Subscription>();
  private operations = new Map<string, Operation>();
  private pending = new Set<Pending>();
  private socket?: WebSocket;
  private opened = false;
  private timer?: ReturnType<typeof setTimeout>;
  private failures = 0;
  private sequence = 0;
  onCommandError?: (
    frame: Extract<LiveFrame, { type: "command_error" }>,
  ) => void;
  constructor(private random: () => number = Math.random) {}
  private id() {
    return `op.${++this.sequence}`;
  }
  subscribe(
    command: SubscriptionCommand,
    frame: Subscription["frame"],
    status?: Subscription["status"],
  ) {
    const previous = this.listeners.get(command.id);
    if (previous) this.release(previous);
    const listener: Subscription = {
      kind: "subscription",
      command,
      frame,
      status,
      failures: 0,
    };
    this.listeners.set(command.id, listener);
    if (this.opened) this.start(listener);
    else {
      status?.("Connecting");
      this.ensureConnected();
    }
    return () => {
      if (this.listeners.get(command.id) !== listener) return;
      this.listeners.delete(command.id);
      this.release(listener);
      this.closeIdle();
    };
  }
  update(command: SubscriptionCommand) {
    const listener = this.listeners.get(command.id);
    if (
      !listener ||
      JSON.stringify(listener.command) === JSON.stringify(command)
    )
      return;
    listener.command = command;
    listener.resume = undefined;
    listener.failures = 0;
    this.restart(listener);
  }
  /** Update the query to resume after transport loss without restarting live work. */
  remember(command: SubscriptionCommand) {
    const listener = this.listeners.get(command.id);
    if (listener) listener.resume = command;
  }
  retry(id: string) {
    const listener = this.listeners.get(id);
    if (listener) {
      listener.failures = 0;
      this.restart(listener);
    }
  }
  request<M extends Call["method"]>(
    call: Extract<Call, { method: M }>,
    signal?: AbortSignal,
  ): Promise<Result<M>> {
    if (signal?.aborted) return Promise.reject(signal.reason);
    if (this.pending.size >= 8)
      return Promise.reject(
        new ApiFailure(
          "capacity_exceeded",
          "Too many pending WebSocket requests",
        ),
      );
    return new Promise((resolve, reject) => {
      const wire = this.id();
      let done = false;
      let operation: Pending;
      let timeout: ReturnType<typeof setTimeout>;
      const finish = (
        error?: unknown,
        result?: RequestResult["data"],
        cancel = false,
      ) => {
        if (done) return;
        done = true;
        clearTimeout(timeout);
        signal?.removeEventListener("abort", abort);
        this.operations.delete(wire);
        this.pending.delete(operation);
        if (cancel && operation.sent) this.send({ type: "cancel", id: wire });
        if (error !== undefined) reject(error);
        else resolve(result as Result<M>);
        this.closeIdle();
      };
      const abort = () =>
        finish(
          signal?.reason ?? new DOMException("Aborted", "AbortError"),
          undefined,
          true,
        );
      operation = { kind: "request", wire, call, sent: false, finish };
      timeout = setTimeout(
        () =>
          finish(
            new ApiFailure("timed_out", "WebSocket request timed out"),
            undefined,
            true,
          ),
        35000,
      );
      this.pending.add(operation);
      this.operations.set(wire, operation);
      signal?.addEventListener("abort", abort, { once: true });
      if (this.opened) this.sendRequest(operation);
      else this.ensureConnected();
    });
  }
  private send(value: components["schemas"]["Command"]) {
    if (!this.opened || !this.socket) return false;
    try {
      this.socket.send(JSON.stringify(value));
      return true;
    } catch {
      this.disconnected(this.socket);
      return false;
    }
  }
  private release(listener: Subscription) {
    clearTimeout(listener.retry);
    listener.retry = undefined;
    if (listener.wire) {
      this.operations.delete(listener.wire);
      this.send({ type: "cancel", id: listener.wire });
      listener.wire = undefined;
    }
  }
  private start(listener: Subscription) {
    if (listener.resume) {
      listener.command = listener.resume;
      listener.resume = undefined;
    }
    const wire = this.id();
    listener.wire = wire;
    this.operations.set(wire, listener);
    this.send({ ...listener.command, id: wire });
  }
  private restart(listener: Subscription) {
    this.release(listener);
    if (this.opened) this.start(listener);
  }
  private ensureConnected() {
    if (!this.socket && !this.timer) this.connect();
  }
  private closeIdle() {
    if (this.listeners.size || this.pending.size || this.statusListeners.size)
      return;
    clearTimeout(this.timer);
    this.timer = undefined;
    this.failures = 0;
    this.publishStatus("unavailable");
    const socket = this.socket;
    this.socket = undefined;
    this.opened = false;
    if (socket) {
      socket.onclose = null;
      socket.close();
    }
  }
  private disconnected(socket: WebSocket) {
    if (this.socket !== socket) return;
    this.socket = undefined;
    this.opened = false;
    socket.onclose = null;
    socket.close();
    this.publishStatus("unavailable");
    this.operations.clear();
    for (const operation of [...this.pending])
      operation.finish(
        new ApiFailure(
          "unavailable",
          "WebSocket disconnected; retry the request",
        ),
      );
    for (const listener of this.listeners.values()) {
      clearTimeout(listener.retry);
      listener.retry = undefined;
      listener.wire = undefined;
      listener.status?.("Reconnecting");
    }
    if (this.listeners.size || this.statusListeners.size)
      this.timer = setTimeout(
        () => this.connect(),
        retryDelay(this.failures++, this.random),
      );
  }
  private connect() {
    if (
      !this.listeners.size &&
      !this.pending.size &&
      !this.statusListeners.size
    )
      return;
    this.timer = undefined;
    let socket: WebSocket;
    try {
      socket = new WebSocket(
        `${location.protocol === "https:" ? "wss:" : "ws:"}//${location.host}/ws`,
      );
    } catch {
      for (const operation of [...this.pending])
        operation.finish(
          new ApiFailure("unavailable", "WebSocket is unavailable"),
        );
      if (this.listeners.size || this.statusListeners.size)
        this.timer = setTimeout(
          () => this.connect(),
          retryDelay(this.failures++, this.random),
        );
      return;
    }
    this.socket = socket;
    this.opened = false;
    for (const listener of this.listeners.values())
      listener.status?.("Connecting");
    socket.onopen = () => {
      if (this.socket !== socket) return;
      this.opened = true;
      for (const listener of this.listeners.values()) {
        if (this.socket !== socket) return;
        this.start(listener);
      }
      for (const operation of this.pending) {
        if (this.socket !== socket) return;
        this.sendRequest(operation);
      }
    };
    socket.onmessage = (event) => {
      if (this.socket !== socket) return;
      let value: LiveFrame;
      try {
        value = parseFrame(event.data);
      } catch {
        this.disconnected(socket);
        return;
      }
      if (value.type === "market.status") {
        this.publishStatus(value.status);
        return;
      }
      if (value.type === "command_error") {
        this.onCommandError?.(value);
        return;
      }
      if (value.type === "error" && !value.id) {
        for (const listener of this.listeners.values()) listener.frame(value);
        for (const operation of [...this.pending])
          operation.finish(
            new ApiFailure(
              value.error.code,
              value.error.message,
              value.error.details ?? undefined,
            ),
          );
        this.disconnected(socket);
        return;
      }
      const operation = this.operations.get(value.id);
      if (!operation) return;
      if (operation.kind === "request") {
        if (value.type === "error")
          operation.finish(
            new ApiFailure(
              value.error.code,
              value.error.message,
              value.error.details ?? undefined,
            ),
          );
        else if (
          value.type === "result" &&
          value.method === operation.call.method
        )
          operation.finish(undefined, value.data);
        else this.disconnected(socket);
        return;
      }
      if (
        value.type === "result" ||
        (value.type === "catalog" &&
          operation.command.type !== "watch_catalog") ||
        ((value.type === "orders.snapshot" ||
          value.type === "orders.changed") &&
          operation.command.type !== "orders.watch")
      ) {
        this.disconnected(socket);
        return;
      }
      operation.frame({ ...value, id: operation.command.id });
      if (this.operations.get(value.id) !== operation) return;
      if (value.type === "error") {
        if (
          [
            "unavailable",
            "capacity_exceeded",
            "restart_read",
            "timed_out",
          ].includes(value.error.code)
        ) {
          clearTimeout(operation.retry);
          operation.retry = setTimeout(
            () => {
              operation.retry = undefined;
              if (
                this.listeners.get(operation.command.id) === operation &&
                this.opened
              )
                this.restart(operation);
            },
            retryDelay(operation.failures++, this.random),
          );
        }
      } else if (value.type !== "status") {
        clearTimeout(operation.retry);
        operation.retry = undefined;
        operation.failures = 0;
        this.failures = 0;
      }
    };
    socket.onerror = () => this.disconnected(socket);
    socket.onclose = () => this.disconnected(socket);
  }
  private sendRequest(operation: Pending) {
    if (operation.sent) return;
    operation.sent = true;
    this.send({ type: "request", id: operation.wire, ...operation.call });
  }
}
export const liveSocket = new LiveSocket();
