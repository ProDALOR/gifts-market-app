import { TaskQueue } from "../resources/task-queue.ts";
import {
  MediaResource,
  MediaHttpError,
  MediaNetworkError,
  retryMedia,
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
type MediaType = "image" | "tgs";
/** Bound transfers independently of rendering. Small symbol stills must keep up with grid overscan. */
const downloads = new TaskQueue(8);
export async function fetchMedia(
  url: string,
  signal: AbortSignal,
  kind: MediaType,
  priority = 0,
) {
  return downloads.run(() => download(url, signal, kind), signal, priority);
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
    type &&
    !(kind === "tgs"
      ? ["application/x-tgsticker", "application/octet-stream"].includes(type)
      : ["image/png", "image/jpeg", "image/webp", "image/svg+xml"].includes(
          type,
        ))
  ) {
    await response.body?.cancel().catch(() => {});
    throw new Error("Invalid media type");
  }
  const limit = type === "image/svg+xml" ? 1024 * 1024 : 256 * 1024;
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
// Successful recoveries have zero retention; fresh entries here are failures only.
export const hasImageFailure = (url: string) => imageRecovery.hasFresh(url);
export const recoverImage = (url: string, signal: AbortSignal) =>
  retryMedia(() => imageRecovery.get(url, signal), signal);
