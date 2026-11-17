import assert from "node:assert/strict";
import { test } from "node:test";
import { ConsoleConnection, ConsoleLog } from "../src/docs/socket-console.ts";

test("console has one explicit connection and ignores callbacks from replaced sockets", () => {
  const sockets: {
    readyState: number;
    onopen?: () => void;
    onmessage?: (event: { data: string }) => void;
    onerror?: () => void;
    onclose?: (event: { code: number; reason: string }) => void;
    send: (text: string) => void;
    close: () => void;
  }[] = [];
  const received: string[] = [],
    sent: string[] = [],
    states: string[] = [];
  let closes = 0;
  const connection = new ConsoleConnection(
    "ws://localhost/ws",
    (state) => states.push(state),
    (text) => received.push(text),
    () => {
      const socket = {
        readyState: 0,
        send: (text: string) => sent.push(text),
        close: () => {
          closes++;
        },
      };
      sockets.push(socket);
      return socket as unknown as WebSocket;
    },
  );
  assert.throws(() => connection.send("{}"), /Connect/);
  connection.connect();
  connection.connect();
  assert.equal(sockets.length, 1);
  const first = sockets[0];
  first.readyState = 1;
  first.onopen!();
  connection.send('{"type":"unsubscribe","id":"offers"}');
  assert.equal(sent.length, 1);
  assert.throws(() => connection.send("{"), SyntaxError);
  assert.throws(
    () => connection.send(JSON.stringify("😀".repeat(20_000))),
    /64 KiB/,
  );
  const late = first.onmessage!;
  first.onclose!({ code: 1013, reason: "slow" });
  connection.connect();
  late({ data: "late" });
  assert.deepEqual(received, []);
  sockets[1].readyState = 1;
  sockets[1].onmessage!({ data: "current" });
  assert.deepEqual(received, ["current"]);
  connection.disconnect();
  assert.equal(closes, 1);
  assert.equal(sockets[1].onmessage, null);
  assert.equal(states.at(-1), "closed");
  assert.equal(sockets.length, 2, "disconnect never reconnects automatically");
});
test("console log bounds both event count and retained text, clips large frames and clears", () => {
  const log = new ConsoleLog();
  for (let i = 0; i < 100; i++) log.push("received", String(i));
  assert.equal(log.entries.length, 60);
  assert.equal(log.count, 100);
  assert.equal(log.entries[0].text, "40");
  for (let i = 0; i < 100; i++)
    log.push("received", "x".repeat(4 * 1024 * 1024));
  assert.ok(
    log.entries.reduce((sum, entry) => sum + entry.text.length * 2, 0) <=
      256 * 1024,
  );
  assert.ok(
    log.entries.every(
      (entry) => entry.clipped && entry.text.includes("clipped"),
    ),
  );
  log.clear();
  assert.equal(log.entries.length, 0);
  assert.equal(log.count, 0);
});
