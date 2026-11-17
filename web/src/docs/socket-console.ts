/** One explicit, same-origin debug connection. Old sockets cannot update a new session. */
export type ConnectionState = "closed" | "connecting" | "open";
export class ConsoleConnection {
  private socket?: WebSocket;
  constructor(
    private url: string,
    private state: (state: ConnectionState, detail?: string) => void,
    private received: (text: string) => void,
    private create: (url: string) => WebSocket = (url) => new WebSocket(url),
  ) {}
  connect() {
    if (this.socket) return;
    this.state("connecting");
    let socket: WebSocket;
    try {
      socket = this.create(this.url);
    } catch (error) {
      this.state("closed", String(error));
      return;
    }
    this.socket = socket;
    socket.onopen = () => {
      if (this.socket === socket) this.state("open");
    };
    socket.onmessage = (event) => {
      if (this.socket === socket && typeof event.data === "string")
        this.received(event.data);
    };
    socket.onerror = () => {
      if (this.socket === socket)
        this.state(
          socket.readyState === 1 ? "open" : "connecting",
          "Connection failed. Check the server, origin policy and network.",
        );
    };
    socket.onclose = (event) => {
      if (this.socket !== socket) return;
      this.socket = undefined;
      this.state(
        "closed",
        `Closed ${event.code}${event.reason ? ` · ${event.reason}` : ""}`,
      );
    };
  }
  send(text: string) {
    if (!this.socket || this.socket.readyState !== 1)
      throw new Error("Connect before sending a command.");
    if (new TextEncoder().encode(text).length > 64 * 1024)
      throw new Error(
        "Commands must fit in the server's default 64 KiB input limit.",
      );
    JSON.parse(text); // Catch malformed JSON locally; query validation belongs to the API.
    this.socket.send(text);
  }
  disconnect() {
    const socket = this.socket;
    this.socket = undefined;
    if (socket) {
      socket.onopen = socket.onmessage = socket.onerror = socket.onclose = null;
      socket.close(1000, "Console disconnected");
    }
    this.state("closed");
  }
}
export type LogEntry = {
  direction: "sent" | "received" | "event";
  text: string;
  clipped: boolean;
  time: string;
};
export class ConsoleLog {
  readonly entries: LogEntry[] = [];
  private characters = 0;
  count = 0;
  push(direction: LogEntry["direction"], value: string): LogEntry {
    const clipped = value.length > 16_384;
    // Copy the short preview so a sliced string cannot retain a multi-megabyte backing buffer.
    const preview = clipped
      ? new TextDecoder().decode(
          new TextEncoder().encode(value.slice(0, 16_384)),
        )
      : value;
    const text = clipped
      ? `${preview}\n… clipped (${value.length.toLocaleString()} characters)`
      : value;
    const entry = {
      direction,
      text,
      clipped,
      time: new Date().toLocaleTimeString(),
    };
    this.entries.push(entry);
    this.characters += text.length;
    this.count++;
    while (this.entries.length > 60 || this.characters * 2 > 256 * 1024)
      this.characters -= this.entries.shift()!.text.length;
    return entry;
  }
  clear() {
    this.entries.length = 0;
    this.characters = 0;
    this.count = 0;
  }
}
