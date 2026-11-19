import { ResourceCache } from "../resources/cache.ts";
import { fetchMedia, MediaHttpError } from "./media-client.ts";

const namespace = "http://www.w3.org/2000/svg";
/** Extract data only. Source SVG markup is never inserted into the document. */
export function decodeOutline(bytes: Uint8Array): string {
  if (!bytes.length || bytes.length > 8192)
    throw new Error("Invalid outline size");
  const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  const doc = new DOMParser().parseFromString(text, "image/svg+xml");
  const svg = doc.documentElement;
  const path = svg.firstElementChild;
  if (
    doc.doctype ||
    svg.localName !== "svg" ||
    svg.namespaceURI !== namespace ||
    svg.getAttribute("viewBox") !== "0 0 512 512" ||
    svg.children.length !== 1 ||
    !path ||
    path.localName !== "path" ||
    path.namespaceURI !== namespace ||
    path.children.length ||
    !["#fff", "#ffffff"].includes(path.getAttribute("fill") || "") ||
    [...svg.attributes].some(
      (attr) => !["xmlns", "viewBox"].includes(attr.name),
    ) ||
    [...path.attributes].some((attr) => !["d", "fill"].includes(attr.name))
  ) {
    throw new Error("Invalid outline SVG");
  }
  const data = path.getAttribute("d");
  if (
    !data ||
    data.length > 4096 ||
    !/^[MmLlHhVvCcSsQqTtAaZz0-9eE+.,\s-]+$/.test(data)
  ) {
    throw new Error("Invalid outline path");
  }
  return data;
}

// Paths are shared by all visible copies. Cache optional misses, too, so collection
// switches don't repeat known 404s. Cancellation is never recorded as a miss.
const outlines = new ResourceCache<string | null>(
  async (url, signal) => {
    try {
      const path = decodeOutline(
        await fetchMedia(url, signal, "placeholder", -10),
      );
      return { value: path, bytes: path.length * 2 };
    } catch (error) {
      signal.throwIfAborted();
      const maxAge =
        error instanceof MediaHttpError && error.status === 404
          ? 60_000
          : Math.min(
              error instanceof MediaHttpError
                ? (error.retryAfterMs ?? 2000)
                : 2000,
              300_000,
            );
      return { value: null, bytes: 1, maxAge };
    }
  },
  256 * 1024,
  4,
  300_000,
);

export const loadOutline = (url: string, signal: AbortSignal) =>
  outlines.get(url, signal, -10);
