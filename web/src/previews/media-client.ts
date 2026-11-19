import { TaskQueue } from "../resources/task-queue.ts";
import { ResourceCache } from "../resources/cache.ts";
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
function retryAfter(response: Response): number | undefined {
  if (![429, 503].includes(response.status)) return;
  const value = response.headers.get("Retry-After");
  const seconds =
    value === null
      ? 2
      : /^\d+$/.test(value)
        ? Number(value)
        : (Date.parse(value) - Date.now()) / 1000;
  return Number.isFinite(seconds) ? Math.max(1, seconds) * 1000 : undefined;
}
export class MediaHttpError extends Error {
  constructor(
    readonly status: number,
    readonly retryAfterMs?: number,
  ) {
    super("Media is unavailable");
    this.name = "MediaHttpError";
  }
}
type MediaType = "image" | "tgs" | "placeholder";
/** Limit active HTTP transfers. Backoff never holds a transfer slot. */
const downloads = new TaskQueue(4);
export async function fetchMedia(
  url: string,
  signal: AbortSignal,
  kind: MediaType,
  priority = 0,
) {
  for (let attempt = 0; ; attempt++) {
    try {
      return await downloads.run(
        () => download(url, signal, kind),
        signal,
        priority,
      );
    } catch (error) {
      signal.throwIfAborted();
      if (
        !(error instanceof MediaHttpError) ||
        error.retryAfterMs === undefined ||
        error.retryAfterMs > 10_000 ||
        attempt >= 2
      )
        throw error;
      // Re-enter the priority queue for each attempt, after releasing network capacity.
      await wait(error.retryAfterMs, signal);
    }
  }
}
async function download(url: string, signal: AbortSignal, kind: MediaType) {
  const response = await fetch(url, { signal });
  if (!response.ok) {
    await response.body?.cancel();
    throw new MediaHttpError(response.status, retryAfter(response));
  }
  const type = response.headers.get("Content-Type")?.split(";")[0];
  if (
    (kind === "placeholder" && !type) ||
    (type &&
      !(kind === "tgs"
        ? type === "application/x-tgsticker"
        : kind === "placeholder"
          ? type === "image/svg+xml"
          : ["image/png", "image/jpeg", "image/webp", "image/svg+xml"].includes(
              type,
            )))
  ) {
    await response.body?.cancel();
    throw new Error("Invalid media type");
  }
  const limit =
    kind === "placeholder"
      ? 8 * 1024
      : type === "image/svg+xml"
        ? 1024 * 1024
        : 256 * 1024;
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
// Native <img loading=lazy> remains the fast path. Failed images use the same
// status-aware retry policy, warming the HTTP cache before one remount.
const imageRecovery = new ResourceCache(
  async (url, signal) => {
    await fetchMedia(url, signal, "image");
    return { value: true, bytes: 1 };
  },
  64,
  4,
  30_000,
);
export const recoverImage = (url: string, signal: AbortSignal) =>
  imageRecovery.get(url, signal);
