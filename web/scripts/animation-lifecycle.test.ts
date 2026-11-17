import assert from "node:assert/strict";
import { test } from "node:test";
import { AnimationLifecycle } from "../src/previews/animation-lifecycle.ts";
import type { PreviewPlayer } from "../src/previews/animation-lifecycle.ts";
function fixture() {
  let creates = 0,
    plays = 0,
    pauses = 0,
    destroys = 0,
    visible = false;
  const listeners = new Map<string, () => void>();
  const player: PreviewPlayer = {
    play: () => {
      plays++;
    },
    pause: () => {
      pauses++;
    },
    destroy: () => {
      destroys++;
    },
    ready: () => true,
    listen: (event, callback) => {
      listeners.set(event, callback);
    },
  };
  const lifecycle = new AnimationLifecycle(
    async () => {
      creates++;
      return player;
    },
    (ready) => {
      visible = ready;
    },
  );
  return {
    lifecycle,
    player,
    listeners,
    state: () => ({ creates, plays, pauses, destroys, visible }),
  };
}
const flush = async () => {
  await Promise.resolve();
  await Promise.resolve();
};
test("brief hover gaps pause and reuse the same renderer, longer gaps release it", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { lifecycle, state } = fixture();
  lifecycle.setActive(true);
  await flush();
  assert.equal(state().visible, true);
  lifecycle.setActive(false);
  assert.equal(state().visible, false);
  assert.equal(state().pauses, 1);
  t.mock.timers.tick(200);
  lifecycle.setActive(true);
  await flush();
  assert.equal(state().creates, 1);
  assert.equal(state().visible, true);
  t.mock.timers.tick(500);
  assert.equal(state().destroys, 0);
  lifecycle.setActive(false);
  t.mock.timers.tick(350);
  assert.equal(state().destroys, 1);
  assert.equal(state().visible, false);
  lifecycle.stop();
  assert.equal(state().destroys, 1);
});
test("offscreen, hidden-tab and reduced-motion stops destroy immediately", async () => {
  const { lifecycle, state } = fixture();
  lifecycle.setActive(true);
  await flush();
  lifecycle.setActive(false, false);
  assert.equal(state().visible, false);
  assert.equal(state().destroys, 1);
  lifecycle.setActive(true);
  await flush();
  assert.equal(state().creates, 2);
  lifecycle.stop();
  assert.equal(state().destroys, 2);
});
for (const event of ["error", "data_failed"])
  test(`${event} restores fallback and releases the renderer`, async () => {
    const { lifecycle, listeners, state } = fixture();
    lifecycle.setActive(true);
    await flush();
    listeners.get(event)!();
    assert.equal(state().visible, false);
    assert.equal(state().destroys, 1);
    listeners.get("DOMLoaded")!();
    assert.equal(state().visible, false);
    lifecycle.stop();
    assert.equal(state().destroys, 1);
  });
test("stopping an in-flight load aborts and destroys a late renderer", async () => {
  let finish: (player: PreviewPlayer) => void = () => {};
  let signal: AbortSignal | undefined,
    destroys = 0,
    shown = false;
  const lifecycle = new AnimationLifecycle(
    async (stop) => {
      signal = stop;
      return await new Promise<PreviewPlayer>((resolve) => {
        finish = resolve;
      });
    },
    (ready) => {
      shown = ready;
    },
  );
  lifecycle.setActive(true);
  lifecycle.stop();
  assert.equal(signal?.aborted, true);
  finish({
    play() {},
    pause() {},
    destroy() {
      destroys++;
    },
    ready: () => true,
    listen() {},
  });
  await flush();
  assert.equal(destroys, 1);
  assert.equal(shown, false);
});
test("synchronous playback failures restore fallback without escaping", async () => {
  let visible = false,
    destroyed = false;
  const lifecycle = new AnimationLifecycle(
    async () => ({
      play() {
        throw Error("canvas");
      },
      pause() {},
      destroy() {
        destroyed = true;
      },
      ready: () => true,
      listen() {},
    }),
    (ready) => {
      visible = ready;
    },
  );
  lifecycle.setActive(true);
  await flush();
  assert.equal(visible, false);
  assert.equal(destroyed, true);
});

test("a synchronous player error event cannot re-enable the failed canvas", async () => {
  let visible = false;
  const listeners = new Map<string, () => void>();
  const lifecycle = new AnimationLifecycle(
    async () => ({
      play() {
        listeners.get("error")?.();
      },
      pause() {},
      destroy() {},
      ready: () => true,
      listen(event, callback) {
        listeners.set(event, callback);
      },
    }),
    (ready) => {
      visible = ready;
    },
  );
  lifecycle.setActive(true);
  await flush();
  assert.equal(visible, false);
});
test("temporary queue saturation retries while active and cancellation prevents later creation", async (t) => {
  const { QueueFullError } = await import("../src/previews/task-queue.ts");
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let attempts = 0;
  const controller = new AnimationLifecycle(
    async () => {
      attempts++;
      throw new QueueFullError();
    },
    () => {},
  );
  controller.setActive(true);
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(attempts, 1);
  t.mock.timers.tick(150);
  assert.equal(attempts, 2);
  controller.setActive(false, false);
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  t.mock.timers.tick(1000);
  assert.equal(attempts, 2);
});
