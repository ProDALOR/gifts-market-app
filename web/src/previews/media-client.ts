import { TaskQueue } from "../resources/task-queue.ts";
import {
  MediaResource,
  MediaHttpError,
  MediaNetworkError,
  retryMedia,
  wait,
} from "./media-resource.ts";
function retryAfter(response: Response): number | undefined {
  const value = response.headers.get("Retry-After");
  if (value === null)
    return [429, 503].includes(response.status) ? 2000 : undefined;
  const seconds = /^\d+$/.test(value)
    ? Number(value)
    : (Date.parse(value) - Date.now()) / 1000;
  return Number.isFinite(seconds) ? Math.max(1, seconds) * 1000 : undefined;
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
        !error.retryable ||
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
  const response = await fetch(url, { signal }).catch(() => {
    signal.throwIfAborted();
    throw new MediaNetworkError("Media download failed");
  });
  if (!response.ok) {
    await response.body?.cancel().catch(() => {});
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
    await response.body?.cancel().catch(() => {});
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
    await response.body?.cancel().catch(() => {});
    throw new Error("Invalid media size");
  }
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    while (true) {
      const chunk = await reader.read().catch(() => {
        signal.throwIfAborted();
        throw new MediaNetworkError("Media download interrupted");
      });
      if (chunk.done) break;
      size += chunk.value.length;
      if (size > limit) throw new Error("Media is too large");
      chunks.push(chunk.value);
    }
  } finally {
    await reader.cancel().catch(() => {});
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
const imageRecovery = new MediaResource(
  async (url, signal) => {
    await fetchMedia(url, signal, "image");
    const image = new Image();
    const abort = () => image.removeAttribute("src");
    signal.addEventListener("abort", abort, { once: true });
    try {
      signal.throwIfAborted();
      image.src = url;
      await image.decode();
      signal.throwIfAborted();
    } finally {
      signal.removeEventListener("abort", abort);
      image.removeAttribute("src");
    }
    return { value: true, bytes: 1 };
  },
  64,
  4,
  0, // Native HTTP caching owns successes; only in-flight work and failures need sharing here.
);
export const recoverImage = (url: string, signal: AbortSignal) =>
  retryMedia(() => imageRecovery.get(url, signal), signal);
