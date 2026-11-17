/** Shared bounded resources: one producer, independent consumers, byte-based LRU. */
export class MediaStore<T> {
  private entries = new Map<string, Entry<T>>();
  private bytes = 0;
  constructor(
    private load: (
      key: string,
      signal: AbortSignal,
    ) => Promise<{ value: T; bytes: number }>,
    private budget = 16 * 1024 * 1024,
    private maxPending = 16,
  ) {}
  async get(key: string, signal: AbortSignal): Promise<T> {
    signal.throwIfAborted();
    let entry = this.entries.get(key);
    if (!entry) {
      if (
        [...this.entries.values()].filter((value) => !value.settled).length >=
        this.maxPending
      )
        throw new Error("Too many media requests");
      const controller = new AbortController();
      const created: Entry<T> = {
        controller,
        users: 0,
        settled: false,
        bytes: 0,
        promise: Promise.resolve(undefined as T),
      };
      this.entries.set(key, created);
      const timeout = setTimeout(() => controller.abort(), 35000);
      created.promise = this.load(key, controller.signal)
        .then(({ value, bytes }) => {
          controller.signal.throwIfAborted();
          if (this.entries.get(key) === created) {
            created.bytes = bytes;
            this.bytes += bytes;
            created.settled = true;
            this.trim();
          }
          return value;
        })
        .catch((error) => {
          this.remove(key, created);
          throw error;
        })
        .finally(() => {
          created.settled = true;
          clearTimeout(timeout);
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
  promise: Promise<T>;
};
export function wait(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const aborted = () => {
      clearTimeout(timer);
      reject(signal.reason);
    };
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", aborted);
      resolve();
    }, ms);
    if (signal.aborted) aborted();
    else signal.addEventListener("abort", aborted, { once: true });
  });
}
export function retryDelay(response: Response): number | undefined {
  if (![429, 503].includes(response.status)) return;
  const value = response.headers.get("Retry-After");
  const seconds =
    value === null
      ? 2
      : /^\d+$/.test(value)
        ? Number(value)
        : (Date.parse(value) - Date.now()) / 1000;
  return Number.isFinite(seconds) && seconds <= 10
    ? Math.max(1, seconds) * 1000
    : undefined;
}
/** Keep native HTTP caching and cap streamed bytes, including chunked responses. */
export async function fetchMedia(
  url: string,
  signal: AbortSignal,
  kind: "image" | "tgs",
) {
  for (let attempt = 0; attempt < 3; attempt++) {
    const response = await fetch(url, { signal });
    if (!response.ok) {
      await response.body?.cancel();
      const delay = retryDelay(response);
      if (delay === undefined || attempt === 2)
        throw new Error("Media is unavailable");
      await wait(delay, signal);
      continue;
    }
    const type = response.headers.get("Content-Type")?.split(";")[0];
    if (
      type &&
      !(kind === "tgs"
        ? type === "application/x-tgsticker"
        : ["image/png", "image/jpeg", "image/webp"].includes(type))
    ) {
      await response.body?.cancel();
      throw new Error("Invalid media type");
    }
    const limit = 256 * 1024;
    if (
      Number(response.headers.get("Content-Length")) > limit ||
      !response.body
    ) {
      await response.body?.cancel();
      throw new Error("Invalid media size");
    }
    const reader = response.body.getReader();
    const chunks: Uint8Array[] = [];
    let size = 0;
    try {
      while (true) {
        const chunk = await reader.read();
        if (chunk.done) break;
        size += chunk.value.length;
        if (size > limit) throw new Error("Media is too large");
        chunks.push(chunk.value);
      }
    } finally {
      await reader.cancel();
    }
    const bytes = new Uint8Array(size);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.length;
    }
    return bytes;
  }
  throw new Error("Media is unavailable");
}
// Native <img loading=lazy> remains the fast path. Failed images use the same
// status-aware retry policy, warming the HTTP cache before one remount.
const imageRecovery = new MediaStore(async (url, signal) => {
  await fetchMedia(url, signal, "image");
  return { value: true, bytes: 1 };
}, 64);
export const recoverImage = (url: string, signal: AbortSignal) =>
  imageRecovery.get(url, signal);
