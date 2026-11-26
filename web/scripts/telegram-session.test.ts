import assert from "node:assert/strict";
import { test } from "node:test";
import type { TestContext } from "node:test";
import { ApiFailure } from "../src/api-error.ts";
import { TelegramSessionController } from "../src/telegram-session.ts";
import type {
  SessionRequest,
  TelegramSessionState,
} from "../src/telegram-session.ts";
import {
  cachedSession,
  launchFingerprint,
  saveSession,
} from "../src/telegram.ts";

const config = {
  bot_id: "42",
  bot_username: "fixture_bot",
  auth_enabled: true,
};
const context = { initData: "launch-one" };
const identity = () => ({
  user: { id: "7", first_name: "Fixture", last_name: null, username: null },
  expires_at: new Date(Date.now() + 86400000).toISOString(),
});
const session = () => ({ ...identity(), token: "signed-session-fixture" });
function setup(t: TestContext) {
  t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 1800000000000 });
  const previous = Object.getOwnPropertyDescriptor(
    globalThis,
    "sessionStorage",
  );
  const values = new Map<string, string>();
  Object.defineProperty(globalThis, "sessionStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
      removeItem: (key: string) => values.delete(key),
    },
  });
  t.after(() => {
    if (previous) Object.defineProperty(globalThis, "sessionStorage", previous);
    else Reflect.deleteProperty(globalThis, "sessionStorage");
  });
}
function controller(
  t: TestContext,
  request: (
    path: string,
    body?: unknown,
    signal?: AbortSignal,
    token?: string,
  ) => Promise<unknown>,
  active?: () => boolean,
  launch = context,
) {
  const value = new TelegramSessionController(
    launch,
    request as SessionRequest,
    active,
  );
  t.after(() => value.stop());
  return value;
}
function when(
  value: TelegramSessionController,
  status: TelegramSessionState["status"],
) {
  if (value.current().status === status) return Promise.resolve();
  return new Promise<void>((resolve) => {
    const off = value.subscribe(() => {
      if (value.current().status === status) {
        off();
        resolve();
      }
    });
  });
}

test("sign-in waits for bot identity, shares one attempt and stops polling after success", async (t) => {
  setup(t);
  let configs = 0,
    signins = 0;
  const value = controller(t, async (path) => {
    if (path === "/telegram")
      return ++configs === 1 ? { auth_enabled: false } : config;
    signins++;
    return session();
  });
  value.start();
  value.start();
  await when(value, "retrying");
  t.mock.timers.tick(999);
  assert.equal(configs, 1);
  t.mock.timers.tick(1);
  await when(value, "connected");
  assert.equal(configs, 2);
  assert.equal(signins, 1);
  value.resume();
  t.mock.timers.tick(60000);
  assert.equal(configs, 2);
  assert.equal(value.current().identity?.user.id, "7");
  assert.equal(
    "token" in value.current().identity!,
    false,
    "rendered identity excludes credentials",
  );
});

test("unavailable configuration has a bounded retry budget and foreground can retry", async (t) => {
  setup(t);
  let calls = 0;
  const value = controller(t, async () => {
    calls++;
    return { auth_enabled: false };
  });
  value.start();
  await when(value, "retrying");
  for (const delay of [1000, 2000, 4000, 8000, 15000, 30000]) {
    t.mock.timers.tick(delay);
    await when(value, calls === 7 ? "unavailable" : "retrying");
  }
  assert.equal(calls, 7);
  assert.equal(value.current().status, "unavailable");
  t.mock.timers.tick(600000);
  assert.equal(calls, 7);
  value.resume();
  await when(value, "retrying");
  assert.equal(calls, 8);
});

test("offline and hidden launches wait for foreground instead of issuing requests", async (t) => {
  setup(t);
  let active = false,
    calls = 0;
  const value = controller(
    t,
    async (path) => {
      calls++;
      return path === "/telegram" ? config : session();
    },
    () => active,
  );
  value.start();
  assert.equal(calls, 0);
  value.resume();
  assert.equal(calls, 0);
  active = true;
  value.resume();
  await when(value, "connected");
  assert.equal(calls, 2);
});

test("timeouts release an attempt even if a transport ignores cancellation", async (t) => {
  setup(t);
  let release!: (value: unknown) => void,
    calls = 0;
  const value = controller(t, async (path) => {
    calls++;
    if (calls === 1)
      return new Promise((resolve) => {
        release = resolve;
      });
    return path === "/telegram" ? config : session();
  });
  value.start();
  t.mock.timers.tick(10000);
  assert.equal(value.current().status, "retrying");
  release(config);
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(calls, 1);
  assert.equal(value.current().identity, undefined);
  t.mock.timers.tick(1000);
  await when(value, "connected");
  assert.equal(calls, 3);
});

test("cached sessions are validated and unauthorized tokens are replaced once", async (t) => {
  setup(t);
  const launch = await launchFingerprint(context);
  saveSession("42", launch, "old-token");
  const paths: string[] = [];
  const value = controller(t, async (path, _body, _stop, token) => {
    paths.push(path);
    if (path === "/telegram") return config;
    if (path === "/auth/me") {
      assert.equal(token, "old-token");
      throw new ApiFailure("unauthorized", "expired");
    }
    return session();
  });
  value.start();
  await when(value, "connected");
  assert.deepEqual(paths, ["/telegram", "/auth/me", "/auth/telegram"]);
  assert.equal(cachedSession("42", launch)?.token, session().token);
});

test("expired identity and rejected launch proofs require reopening, never retry old proof", async (t) => {
  setup(t);
  let calls = 0;
  const value = controller(t, async (path) => {
    calls++;
    return path === "/telegram"
      ? config
      : { ...session(), expires_at: new Date(Date.now() + 1000).toISOString() };
  });
  value.start();
  await when(value, "connected");
  t.mock.timers.tick(1000);
  assert.equal(value.current().status, "reopen");
  assert.equal(value.current().identity, undefined);
  value.stop();
  value.start();
  value.retry();
  value.resume();
  t.mock.timers.tick(60000);
  assert.equal(calls, 2);
  assert.equal(cachedSession("42", await launchFingerprint(context)), null);

  const rejected = controller(t, async (path) => {
    if (path === "/telegram") return config;
    throw new ApiFailure("unauthorized", "invalid or expired proof");
  });
  rejected.start();
  await when(rejected, "reopen");
  rejected.retry();
  t.mock.timers.tick(60000);
  assert.equal(rejected.current().status, "reopen");
});

test("sign-out aborts pending work and pauses only the current launch", async (t) => {
  setup(t);
  let release!: (value: unknown) => void;
  const value = controller(t, async (path) =>
    path === "/telegram"
      ? config
      : new Promise((resolve) => {
          release = resolve;
        }),
  );
  value.start();
  // The sign-in request itself starts after the asynchronous launch fingerprint.
  while (!release) await new Promise((resolve) => setImmediate(resolve));
  value.logout();
  release(session());
  await Promise.resolve();
  await Promise.resolve();
  t.mock.timers.tick(60000);
  assert.equal(value.current().status, "signed_out");
  assert.equal(value.current().identity, undefined);
  assert.equal(cachedSession("42", await launchFingerprint(context)), null);
  let signins = 0;
  const resumed = controller(t, async (path) => {
    if (path === "/telegram") return config;
    signins++;
    return session();
  });
  resumed.start();
  await when(resumed, "signed_out");
  assert.equal(signins, 0);
  const other = controller(
    t,
    async (path) => (path === "/telegram" ? config : session()),
    undefined,
    { initData: "another-launch" },
  );
  other.start();
  await when(other, "connected");
});

test("stopped or superseded reads cannot overwrite a new launch's token", async (t) => {
  setup(t);
  let release!: (value: unknown) => void;
  const old = controller(
    t,
    async () =>
      new Promise((resolve) => {
        release = resolve;
      }),
  );
  old.start();
  old.stop();
  const next = controller(
    t,
    async (path) => (path === "/telegram" ? config : session()),
    undefined,
    { initData: "new-launch" },
  );
  next.start();
  await when(next, "connected");
  release(config);
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(old.current().identity, undefined);
  assert.equal(
    cachedSession("42", await launchFingerprint({ initData: "new-launch" }))
      ?.token,
    session().token,
  );
});

test("foreground events after disposal cannot expire a replacement session", async (t) => {
  setup(t);
  const value = controller(t, async (path) =>
    path === "/telegram" ? config : session(),
  );
  value.start();
  await when(value, "connected");
  value.stop();
  const launch = await launchFingerprint(context);
  saveSession("42", launch, "replacement-session");
  t.mock.timers.tick(86400001);
  value.resume();
  assert.equal(cachedSession("42", launch)?.token, "replacement-session");
});
