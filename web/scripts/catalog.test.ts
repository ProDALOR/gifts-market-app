import assert from "node:assert/strict";
import { test } from "node:test";
import type { TestContext } from "node:test";
import { CatalogController } from "../src/catalog/controller.ts";
import { ApiFailure } from "../src/market.ts";
import type { Catalog, CollectionCatalog } from "../src/market.ts";
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
async function until(predicate: () => boolean) {
  for (let i = 0; i < 100; i++) {
    if (predicate()) return;
    await tick();
  }
  assert.fail("Catalog did not settle");
}
function gate() {
  let release: () => void = () => {};
  const promise = new Promise<void>((r) => {
    release = r;
  });
  return { promise, release };
}
function fixture(t: TestContext) {
  const definitions = new Map<
    string,
    { name: string; version: string; count: number }
  >([
    ["a", { name: "A", version: "a1", count: 2 }],
    ["b", { name: "B", version: "b1", count: 1 }],
  ]);
  let directoryVersion = "d1";
  const requests: { kind: string; id: string; signal: AbortSignal }[] = [];
  let waitDirectory: (() => Promise<void>) | undefined,
    waitCollection: ((id: string) => Promise<void>) | undefined;
  let failure: Error | undefined;
  let callback: ((frame: any) => void) | undefined;
  let status: ((state: string) => void) | undefined;
  let command: any;
  const watchCommands: any[] = [];
  let retries = 0;
  const emit = () =>
    callback?.({
      type: "catalog.versions",
      id: "catalog",
      versions: {
        directory: directoryVersion,
        collections: Object.fromEntries(
          (command?.collection_ids ?? []).map((id: string) => [
            id,
            definitions.get(id)?.version ?? null,
          ]),
        ),
      },
    });
  const socket = {
    subscribe(
      cmd: any,
      frame: (frame: any) => void,
      changed?: (status: string) => void,
    ) {
      command = cmd;
      watchCommands.push(cmd);
      callback = frame;
      status = changed;
      queueMicrotask(emit);
      return () => {
        callback = undefined;
      };
    },
    update(cmd: any) {
      command = cmd;
      watchCommands.push(cmd);
      queueMicrotask(emit);
    },
    retry() {
      retries++;
      queueMicrotask(emit);
    },
  };
  const controller = new CatalogController({
    socket,
    random: () => 0.5,
    catalog: async (signal, received) => {
      requests.push({ kind: "directory", id: "", signal });
      const version = directoryVersion;
      const collections = [...definitions].map(([id, v]) => ({
        id,
        name: v.name,
        slug: null,
        preview: null,
      }));
      const result: Catalog = {
        collections,
        venues: [],
        currencies: [],
        books: collections.map((c) => ({
          collection_id: c.id,
          venue_id: "test",
          currency_id: "GRAM",
        })),
      };
      await waitDirectory?.();
      received?.(version);
      return result;
    },
    collection: async (id, signal, received) => {
      requests.push({ kind: "choices", id, signal });
      const data = definitions.get(id)!;
      const version = data.version;
      const result: CollectionCatalog = {
        collection: { id, name: data.name, slug: null, preview: null },
        models: Array.from({ length: data.count }, (_, i) => ({
          id: `${id}.${i}`,
          name: version,
          media: null,
        })),
        backdrops: [],
        patterns: [],
      };
      await waitCollection?.(id);
      if (failure) {
        const error = failure;
        failure = undefined;
        throw error;
      }
      received?.(version);
      return result;
    },
  });
  t.after(() => controller.close());
  const start = async (id = "a") => {
    controller.select(id);
    controller.start();
    await until(
      () =>
        !!controller.getSnapshot().options.size &&
        !controller.getSnapshot().optionsLoading,
    );
  };
  return {
    controller,
    start,
    definitions,
    requests,
    watchCommands,
    emit,
    directory: (version: string) => {
      directoryVersion = version;
    },
    holdDirectory: (wait?: () => Promise<void>) => {
      waitDirectory = wait;
    },
    holdCollection: (wait?: (id: string) => Promise<void>) => {
      waitCollection = wait;
    },
    fail: (error: Error) => {
      failure = error;
    },
    error: (code: string) =>
      callback?.({
        type: "error",
        id: "catalog",
        error: { code, message: "catalog failed" },
      }),
    status: (value: string) => status?.(value),
    watched: () => command.collection_ids,
    retries: () => retries,
  };
}
const choiceCalls = (f: ReturnType<typeof fixture>, id = "a") =>
  f.requests.filter((r) => r.kind === "choices" && r.id === id).length;
test("initial manifest avoids duplicate reads and cached switching preserves options", async (t) => {
  const f = fixture(t);
  await f.start();
  assert.equal(f.watchCommands.length, 1);
  assert.deepEqual(f.watchCommands[0].collection_ids, ["a"]);
  const a = f.controller.getSnapshot().options;
  assert.equal(f.requests.length, 2);
  f.controller.select("b");
  await until(() => f.controller.getSnapshot().options.has("b.0"));
  f.controller.select("a");
  await until(() => f.controller.getSnapshot().options === a);
  assert.equal(choiceCalls(f), 1);
  assert.equal(choiceCalls(f, "b"), 1);
  f.emit();
  await tick();
  assert.equal(f.requests.length, 3);
});
test("renaming a selected collection does not cancel or duplicate its choices download", async (t) => {
  const f = fixture(t);
  await f.start();
  const pending = gate();
  f.holdCollection(() => pending.promise);
  f.definitions.get("a")!.name = "Renamed";
  f.definitions.get("a")!.version = "a2";
  f.directory("d2");
  f.emit();
  await until(() => f.controller.getSnapshot().item?.name === "Renamed");
  assert.equal(choiceCalls(f), 2);
  assert.equal(
    f.requests
      .filter((r) => r.kind === "choices")
      .some((r) => r.signal.aborted),
    false,
  );
  pending.release();
  await until(
    () => f.controller.getSnapshot().options.get("a.0")?.name === "a2",
  );
  assert.equal(choiceCalls(f), 2);
});
test("directory bursts coalesce behind one request and fetch the latest target once", async (t) => {
  const f = fixture(t);
  await f.start();
  const pending = gate();
  f.holdDirectory(() => pending.promise);
  for (let i = 2; i < 8; i++) {
    f.directory(`d${i}`);
    f.definitions.get("a")!.name = `A${i}`;
    f.emit();
  }
  assert.equal(f.requests.filter((r) => r.kind === "directory").length, 2);
  f.holdDirectory();
  pending.release();
  await until(() => f.controller.getSnapshot().item?.name === "A7");
  assert.equal(f.requests.filter((r) => r.kind === "directory").length, 3);
  assert.equal(
    f.requests.some((r) => r.signal.aborted),
    false,
  );
  assert.equal(choiceCalls(f), 1);
});
test("choice invalidations coalesce and an obsolete response cannot overwrite current data", async (t) => {
  const f = fixture(t);
  await f.start();
  const pending = gate();
  f.holdCollection(() => pending.promise);
  f.definitions.get("a")!.version = "a2";
  f.emit();
  f.definitions.get("a")!.version = "a3";
  f.emit();
  assert.equal(choiceCalls(f), 2);
  assert.equal(f.controller.getSnapshot().options.get("a.0")?.name, "a1");
  f.holdCollection();
  pending.release();
  await until(
    () => f.controller.getSnapshot().options.get("a.0")?.name === "a3",
  );
  assert.equal(choiceCalls(f), 3);
});
test("a newer target recovers an already-stale entry after a non-retried refresh error", async (t) => {
  const f = fixture(t);
  await f.start();
  f.fail(new ApiFailure("not_found", "temporarily missing"));
  f.definitions.get("a")!.version = "a2";
  f.emit();
  await until(() => !!f.controller.getSnapshot().optionsError);
  assert.equal(f.controller.getSnapshot().options.get("a.0")?.name, "a1");
  f.definitions.get("a")!.version = "a3";
  f.emit();
  await until(
    () => f.controller.getSnapshot().options.get("a.0")?.name === "a3",
  );
  assert.equal(f.controller.getSnapshot().optionsError, "");
});
test("transient retries survive unrelated updates without starting an immediate retry loop", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const f = fixture(t);
  await f.start();
  f.fail(new ApiFailure("capacity_exceeded", "busy"));
  f.definitions.get("a")!.version = "a2";
  f.emit();
  await until(() => !!f.controller.getSnapshot().optionsError);
  f.directory("d2");
  f.emit();
  await tick();
  assert.equal(choiceCalls(f), 2);
  t.mock.timers.tick(999);
  await tick();
  assert.equal(choiceCalls(f), 2);
  t.mock.timers.tick(1);
  await until(
    () => f.controller.getSnapshot().options.get("a.0")?.name === "a2",
  );
  assert.equal(choiceCalls(f), 3);
});
test("inactive changes stay lazy and reconnects preserve matching snapshots", async (t) => {
  const f = fixture(t);
  await f.start();
  f.controller.select("b");
  await until(() => !!f.controller.getSnapshot().options.get("b.0"));
  f.controller.select("a");
  const a = f.controller.getSnapshot().options;
  f.definitions.get("b")!.version = "b2";
  f.emit();
  await tick();
  assert.equal(choiceCalls(f, "b"), 1);
  f.status("Reconnecting");
  f.emit();
  await tick();
  assert.equal(f.controller.getSnapshot().options, a);
  assert.equal(choiceCalls(f), 1);
  f.controller.select("b");
  await until(
    () => f.controller.getSnapshot().options.get("b.0")?.name === "b2",
  );
  assert.equal(choiceCalls(f, "b"), 2);
});
test("A to B to A reuses pending work and late inactive data never changes the selection", async (t) => {
  const f = fixture(t);
  await f.start();
  const a = gate(),
    b = gate();
  f.holdCollection((id) => (id === "a" ? a.promise : b.promise));
  f.definitions.get("a")!.version = "a2";
  f.emit();
  f.controller.select("b");
  await until(() => choiceCalls(f, "b") === 1);
  f.controller.select("a");
  assert.equal(choiceCalls(f), 2);
  a.release();
  await until(
    () => f.controller.getSnapshot().options.get("a.0")?.name === "a2",
  );
  b.release();
  await tick();
  await tick();
  assert.equal(f.controller.getSnapshot().selectedId, "a");
  assert.equal(choiceCalls(f), 2);
  assert.equal(
    f.requests.some((r) => r.signal.aborted),
    false,
  );
});
test("terminal live errors are visible and targeted recovery preserves valid HTTP data", async (t) => {
  const f = fixture(t);
  await f.start();
  const before = f.requests.length;
  f.error("invalid_query");
  assert.match(f.controller.getSnapshot().startupError, /catalog failed/);
  f.controller.retryCatalog();
  await until(() => !f.controller.getSnapshot().startupError);
  assert.equal(f.retries(), 1);
  assert.equal(
    f.requests.length,
    before,
    "Live recovery preserves valid HTTP data",
  );
  f.controller.retryOptions();
  await until(
    () =>
      f.requests.length === before + 1 &&
      !f.controller.getSnapshot().optionsLoading,
  );
});
test("LRU retention stays bounded and oversized active collections remain live", async (t) => {
  const f = fixture(t);
  for (let i = 0; i < 9; i++)
    f.definitions.set(`gift${i}`, {
      name: `Gift${i}`,
      version: `v${i}`,
      count: 1,
    });
  await f.start();
  for (let i = 0; i < 9; i++) {
    f.controller.select(`gift${i}`);
    await until(() => f.controller.getSnapshot().options.has(`gift${i}.0`));
  }
  assert.ok(f.watched().length <= 8);
  f.controller.select("a");
  await until(() => f.controller.getSnapshot().options.has("a.0"));
  assert.equal(choiceCalls(f), 2);
  f.definitions.get("a")!.count = 10001;
  f.definitions.get("a")!.version = "large1";
  f.emit();
  await until(() => f.controller.getSnapshot().options.size === 10001);
  f.definitions.get("a")!.version = "large2";
  f.emit();
  await until(
    () => f.controller.getSnapshot().options.get("a.0")?.name === "large2",
  );
  assert.ok(f.watched().length <= 8);
});
test("closing aborts pending requests and prevents late results or retries from publishing", async (t) => {
  const f = fixture(t);
  await f.start();
  const pending = gate();
  f.holdCollection(() => pending.promise);
  f.definitions.get("a")!.version = "a2";
  f.emit();
  const snapshot = f.controller.getSnapshot();
  f.controller.close();
  assert.equal(f.requests.at(-1)!.signal.aborted, true);
  pending.release();
  await tick();
  await tick();
  assert.equal(f.controller.getSnapshot(), snapshot);
});

test("rapid switching bounds in-flight reads and discarded responses cannot publish", async (t) => {
  const f = fixture(t);
  f.definitions.set("c", { name: "C", version: "c1", count: 1 });
  await f.start();
  const a = gate(),
    b = gate();
  f.holdCollection((id) =>
    id === "a" ? a.promise : id === "b" ? b.promise : Promise.resolve(),
  );
  f.definitions.get("a")!.version = "a2";
  f.emit();
  f.controller.select("b");
  await until(() => choiceCalls(f, "b") === 1);
  f.controller.select("c");
  await until(() => f.controller.getSnapshot().options.has("c.0"));
  assert.equal(
    f.requests.filter((r) => r.kind === "choices" && r.id === "a").at(-1)!
      .signal.aborted,
    true,
  );
  a.release();
  b.release();
  await tick();
  await tick();
  assert.equal(f.controller.getSnapshot().selectedId, "c");
  assert.equal(f.controller.getSnapshot().options.has("c.0"), true);
});

test("compact tokens retain selective refreshes and recover missed changes on reconnect", async (t) => {
  const f = fixture(t);
  const directory = "RCdlHdPhcr-iTt64ylBkPA";
  const a = "_y_lqMGEh6UieJB3_3_TTw";
  const b = "EnnK_4Kr2qgEkLGpYM3NxA";
  const updated = "a2aLbICR6oEhXOMFiOrwxw";
  const latest = "O7mGrcSR3nu-zTm_kIaNJw";
  f.directory(directory);
  f.definitions.get("a")!.version = a;
  f.definitions.get("b")!.version = b;
  await f.start();
  const selected = f.controller.getSnapshot().options;
  f.controller.select("b");
  await until(() => f.controller.getSnapshot().options.get("b.0")?.name === b);
  f.controller.select("a");
  const count = f.requests.length;
  f.status("Reconnecting");
  f.emit();
  await tick();
  assert.equal(f.controller.getSnapshot().options, selected);
  assert.equal(
    f.requests.length,
    count,
    "Unchanged reconnects need no HTTP reads",
  );
  // An inactive collection changes while offline; the next manifest marks it stale.
  f.status("Reconnecting");
  f.definitions.get("b")!.version = updated;
  f.emit();
  await tick();
  assert.equal(f.requests.length, count);
  f.controller.select("b");
  await until(
    () => f.controller.getSnapshot().options.get("b.0")?.name === updated,
  );
  assert.equal(choiceCalls(f, "b"), 2);
  assert.equal(choiceCalls(f, "a"), 1);
  assert.equal(f.requests.filter((r) => r.kind === "directory").length, 1);
  // An active collection updates once; a repeated manifest reuses the fresh value.
  f.definitions.get("b")!.version = latest;
  f.emit();
  await until(
    () => f.controller.getSnapshot().options.get("b.0")?.name === latest,
  );
  assert.equal(choiceCalls(f, "b"), 3);
  f.emit();
  await tick();
  assert.equal(choiceCalls(f, "b"), 3);
  f.definitions.delete("b");
  f.emit();
  await until(() => !!f.controller.getSnapshot().optionsError);
  assert.match(f.controller.getSnapshot().optionsError, /no longer available/);
  assert.equal(choiceCalls(f, "b"), 3);
});

test("mixed portfolio previews share bounded reads, live versions and the selected picker cache", async (t) => {
  const ids = Array.from({ length: 40 }, (_, i) => `c${i}`);
  const revisions = new Map(ids.map((id) => [id, "v1"]));
  const watches = new Map<
    string,
    { command: any; frame: (value: any) => void }
  >();
  const emit = () => {
    for (const [id, watch] of watches)
      watch.frame({
        type: "catalog.versions",
        id,
        versions: {
          directory: "directory",
          collections: Object.fromEntries(
            watch.command.collection_ids.map((key: string) => [
              key,
              revisions.get(key),
            ]),
          ),
        },
      });
  };
  const pending = gate();
  let held = true,
    running = 0,
    maximum = 0,
    directories = 0;
  const calls: string[] = [];
  const controller = new CatalogController({
    socket: {
      subscribe(command, frame) {
        watches.set(command.id, { command, frame });
        queueMicrotask(emit);
        return () => {
          watches.delete(command.id);
        };
      },
      update(command) {
        watches.get(command.id)!.command = command;
        queueMicrotask(emit);
      },
      retry() {
        queueMicrotask(emit);
      },
    },
    catalog: async () => {
      directories++;
      return {
        collections: ids.map((id) => ({
          id,
          name: id,
          slug: null,
          preview: null,
        })),
        currencies: [],
        venues: [],
        books: [],
      };
    },
    collection: async (id, signal, received) => {
      running++;
      maximum = Math.max(maximum, running);
      calls.push(id);
      try {
        if (held) await pending.promise;
        assert(!signal.aborted);
        received?.(revisions.get(id)!);
        return {
          collection: { id, name: id, slug: null, preview: null },
          models: [
            { id: `${id}.model`, name: revisions.get(id)!, media: null },
          ],
          backdrops: [],
          patterns: [],
        };
      } finally {
        running--;
      }
    },
  });
  t.after(() => controller.close());
  controller.previews(ids);
  controller.start();
  await until(() => running === 2);
  assert.equal(
    calls.length,
    2,
    "remaining preview reads wait instead of cancelling or flooding",
  );
  held = false;
  pending.release();
  await until(() => controller.getSnapshot().previewOptions.size === 32);
  assert.equal(maximum, 2);
  assert.equal(calls.length, 32, "only the demanded bounded subset is loaded");
  assert.equal(directories, 1);
  assert.equal(watches.size, 4);
  assert(
    [...watches.values()].every((w) => w.command.collection_ids.length <= 8),
  );
  const previous = calls.length;
  controller.select("c2");
  await until(() => controller.getSnapshot().options.has("c2.model"));
  assert.equal(
    calls.length,
    previous,
    "opening a loaded collection reuses its preview request",
  );
  revisions.set("c1", "v2");
  emit();
  await until(
    () =>
      controller.getSnapshot().previewOptions.get("c1.model")?.name === "v2",
  );
  assert.equal(
    calls.length,
    previous + 1,
    "an invalidation rereads only that collection",
  );
  assert.equal(
    controller.getSnapshot().options.size,
    1,
    "picker choices stay collection-scoped",
  );
  assert.equal(controller.getSnapshot().names.get("c1.model"), "v2");
  const first = watches.get("catalog")!,
    other = watches.get("catalog.1")!;
  first.frame({
    type: "error",
    id: "catalog",
    error: { code: "unavailable", message: "first group disconnected" },
  });
  other.frame({
    type: "catalog.versions",
    id: "catalog.1",
    versions: { directory: "directory", collections: {} },
  });
  assert.match(
    controller.getSnapshot().startupError,
    /first group disconnected/,
    "another group's success cannot hide a failed watch",
  );
  emit();
  await until(() => !controller.getSnapshot().startupError);
  controller.close();
  assert.equal(watches.size, 0);
});

test("all collections use shared alias choices without requesting collection models", async (t) => {
  let choiceReads = 0;
  const controller = new CatalogController({
    random: () => 0.5,
    catalog: async (_signal, received) => {
      received?.("shared-v1");
      return {
        collections: [],
        books: [],
        venues: [],
        currencies: [],
        backdrops: [
          {
            id: "native-black",
            name: "Black",
            palette: null,
            aliases: [{ namespace: "name", id: "Black" }],
          },
        ],
        patterns: [
          {
            id: "native-crown",
            name: "Crown",
            media: null,
            aliases: [{ namespace: "name", id: "Crown" }],
          },
        ],
      };
    },
    collection: async () => {
      choiceReads++;
      throw new Error("unexpected collection read");
    },
    socket: {
      subscribe: (_command, frame) => {
        queueMicrotask(() =>
          frame({
            type: "catalog.versions",
            id: "catalog",
            versions: { directory: "shared-v1", collections: {} },
          }),
        );
        return () => {};
      },
      update: () => {},
      retry: () => {},
    },
  });
  t.after(() => controller.close());
  controller.start();
  await until(() => controller.getSnapshot().options.has("@name:Black"));
  const snapshot = controller.getSnapshot();
  assert.equal(snapshot.item?.name, "All gifts");
  assert.equal(snapshot.options.get("@name:Crown")?.kind, "pattern");
  assert.equal(snapshot.optionsLoading, false);
  assert.equal(
    [...snapshot.options.values()].filter((row) => row.kind === "model").length,
    0,
  );
  assert.equal(choiceReads, 0);
});
