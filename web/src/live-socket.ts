import type { components } from "./gen/api.ts";
import { retryDelay } from "./retry.ts";
export type LiveFrame = components["schemas"]["ServerFrame"];
type Command = Exclude<
  components["schemas"]["Command"],
  { type: "unsubscribe" }
>;
type Listener = {
  command: Command;
  frame: (value: LiveFrame) => void;
  status?: (value: string) => void;
  failures: number;
  retry?: ReturnType<typeof setTimeout>;
};
/** Multiplex independent subscriptions. Only transport failures reconnect the whole socket. */
export class LiveSocket {
  private listeners = new Map<string, Listener>();
  private socket?: WebSocket;
  private opened = false;
  private timer?: ReturnType<typeof setTimeout>;
  private failures = 0;
  constructor(private random: () => number = Math.random) {}
  subscribe(
    command: Command,
    frame: Listener["frame"],
    status?: Listener["status"],
  ) {
    const previous = this.listeners.get(command.id);
    if (previous) {
      clearTimeout(previous.retry);
      this.send({ type: "unsubscribe", id: command.id });
    }
    const listener: Listener = {
      command,
      frame,
      status,
      failures: 0,
    };
    this.listeners.set(command.id, listener);
    if (this.opened) this.send(command);
    else if (!this.socket && !this.timer) this.connect();
    return () => {
      if (this.listeners.get(command.id) !== listener) return;
      clearTimeout(listener.retry);
      this.listeners.delete(command.id);
      this.send({ type: "unsubscribe", id: command.id });
      if (!this.listeners.size) {
        clearTimeout(this.timer);
        this.timer = undefined;
        this.failures = 0;
        const socket = this.socket;
        this.socket = undefined;
        this.opened = false;
        if (socket) {
          socket.onclose = null;
          socket.close();
        }
      }
    };
  }
  update(command: Command) {
    const listener = this.listeners.get(command.id);
    if (
      !listener ||
      JSON.stringify(listener.command) === JSON.stringify(command)
    )
      return;
    listener.command = command;
    listener.failures = 0;
    this.restart(listener);
  }
  retry(id: string) {
    const listener = this.listeners.get(id);
    if (listener) {
      listener.failures = 0;
      this.restart(listener);
    }
  }
  private send(value: components["schemas"]["Command"]) {
    if (this.opened) this.socket?.send(JSON.stringify(value));
  }
  private restart(listener: Listener) {
    clearTimeout(listener.retry);
    listener.retry = undefined;
    this.send({ type: "unsubscribe", id: listener.command.id });
    this.send(listener.command);
  }
  private connect() {
    if (!this.listeners.size) return;
    this.timer = undefined;
    const socket = new WebSocket(
      `${location.protocol === "https:" ? "wss:" : "ws:"}//${location.host}/ws`,
    );
    this.socket = socket;
    this.opened = false;
    for (const listener of this.listeners.values())
      listener.status?.("Connecting");
    socket.onopen = () => {
      if (this.socket !== socket) return;
      this.opened = true;
      for (const listener of this.listeners.values()) {
        this.send(listener.command);
      }
    };
    socket.onmessage = (event) => {
      if (this.socket !== socket) return;
      let value: LiveFrame;
      try {
        value = JSON.parse(event.data);
      } catch {
        socket.close();
        return;
      }
      if (value.type === "error" && !value.id) {
        for (const listener of this.listeners.values()) listener.frame(value);
        if (["unavailable", "capacity_exceeded"].includes(value.error.code))
          socket.close();
        return;
      }
      const listener = this.listeners.get(value.id);
      if (!listener) return;
      if (value.type === "error") {
        listener.frame(value);
        if (
          ["unavailable", "capacity_exceeded", "restart_read"].includes(
            value.error.code,
          )
        ) {
          clearTimeout(listener.retry);
          listener.retry = setTimeout(
            () => {
              listener.retry = undefined;
              if (this.listeners.get(value.id) === listener && this.opened)
                this.restart(listener);
            },
            retryDelay(listener.failures++, this.random),
          );
        }
      } else {
        listener.frame(value);
        if (value.type !== "status") {
          listener.failures = 0;
          this.failures = 0;
        }
      }
    };
    socket.onerror = () => socket.close();
    socket.onclose = () => {
      if (this.socket !== socket) return;
      this.socket = undefined;
      this.opened = false;
      for (const listener of this.listeners.values()) {
        clearTimeout(listener.retry);
        listener.retry = undefined;
        listener.status?.("Reconnecting");
      }
      if (this.listeners.size)
        this.timer = setTimeout(
          () => this.connect(),
          retryDelay(this.failures++, this.random),
        );
    };
  }
}
export const liveSocket = new LiveSocket();
