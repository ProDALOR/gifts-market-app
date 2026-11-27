import { TaskQueue } from "./task-queue.ts";
/** Shared bounded resources: queued producers, independent consumers, byte-based LRU. */
export class ResourceCache<T> {
  private entries = new Map<string, Entry<T>>();
  private bytes = 0;
  private queue: TaskQueue;
  constructor(
    private load: (
      key: string,
      signal: AbortSignal,
      priority: number,
    ) => Promise<{ value: T; bytes: number }>,
    private budget = 16 * 1024 * 1024,
    concurrency = 4,
    private maxAge = Infinity,
    private now: () => number = () => Date.now(),
    private failure?: (
      error: unknown,
    ) => { error: unknown; expires: number } | undefined,
  ) {
    this.queue = new TaskQueue(concurrency);
  }
  async get(key: string, signal: AbortSignal, priority = 0): Promise<T> {
    signal.throwIfAborted();
    let entry = this.entries.get(key);
    if (entry?.settled && entry.expires <= this.now()) {
      this.remove(key, entry);
      entry = undefined;
    }
    if (!entry) {
      const controller = new AbortController();
      const created: Entry<T> = {
        controller,
        users: 0,
        settled: false,
        bytes: 0,
        expires: Infinity,
        promise: Promise.resolve(undefined as T),
      };
      this.entries.set(key, created);
      created.promise = this.queue
        .run(
          async () => {
            const timeout = setTimeout(
              () =>
                controller.abort(
                  new DOMException("Resource load timed out", "TimeoutError"),
                ),
              35000,
            );
            try {
              return await this.load(key, controller.signal, priority);
            } finally {
              clearTimeout(timeout);
            }
          },
          controller.signal,
          priority,
        )
        .then(({ value, bytes }) => {
          controller.signal.throwIfAborted();
          if (this.entries.get(key) === created) {
            created.value = value;
            created.expires = this.now() + this.maxAge;
            created.bytes = bytes;
            this.bytes += bytes;
            created.settled = true;
            this.trim();
          }
          return value;
        })
        .catch((error) => {
          const timedOut =
            controller.signal.reason instanceof DOMException &&
            controller.signal.reason.name === "TimeoutError";
          const failure =
            (!controller.signal.aborted || timedOut) &&
            this.failure?.(timedOut ? controller.signal.reason : error);
          if (failure && this.entries.get(key) === created) {
            created.expires = failure.expires;
            created.settled = true;
            created.bytes = 1;
            this.bytes++;
            this.trim();
            throw failure.error;
          }
          this.remove(key, created);
          throw error;
        })
        .finally(() => {
          created.settled = true;
        });
      entry = created;
    } else {
      this.entries.delete(key);
      this.entries.set(key, entry);
    }
    entry.users++;
    let aborted = () => {};
    const cancelled = new Promise<never>((_, reject) => {
      aborted = () => reject(signal.reason);
      signal.addEventListener("abort", aborted, { once: true });
      if (signal.aborted) aborted();
    });
    try {
      return await Promise.race([entry.promise, cancelled]);
    } finally {
      signal.removeEventListener("abort", aborted);
      entry.users--;
      if (!entry.settled && entry.users === 0) {
        entry.controller.abort();
        this.remove(key, entry);
      }
    }
  }
  /** Read an already decoded success without an asynchronous handoff or a new producer. */
  peek(key: string): T | undefined {
    const entry = this.entries.get(key);
    if (!entry?.settled) return;
    if (entry.expires <= this.now()) {
      this.remove(key, entry);
      return;
    }
    if (entry.value === undefined) return;
    this.entries.delete(key);
    this.entries.set(key, entry);
    return entry.value;
  }
  hasFresh(key: string) {
    const entry = this.entries.get(key);
    return !!entry?.settled && entry.expires > this.now();
  }
  invalidate(key: string) {
    const entry = this.entries.get(key);
    if (entry?.settled) this.remove(key, entry);
  }
  private remove(key: string, entry: Entry<T>) {
    if (this.entries.get(key) !== entry) return;
    this.entries.delete(key);
    this.bytes -= entry.bytes;
  }
  private trim() {
    for (const [key, entry] of this.entries) {
      if (this.bytes <= this.budget && this.entries.size <= 64) break;
      if (entry.settled) this.remove(key, entry);
    }
  }
}
type Entry<T> = {
  controller: AbortController;
  users: number;
  settled: boolean;
  bytes: number;
  expires: number;
  promise: Promise<T>;
  value?: T;
};
