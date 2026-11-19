import { MediaResource, MediaFailure } from "./media-resource.ts";
import { fetchMedia } from "./media-client.ts";

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

// Paths and optional failures use the same retention/retry policy as other media.
const outlines = new MediaResource<string>(
  async (url, signal) => {
    const path = decodeOutline(
      await fetchMedia(url, signal, "placeholder", -10),
    );
    return { value: path, bytes: path.length * 2 };
  },
  256 * 1024,
  4,
  300_000,
);

export const outlineCached = (url: string) => outlines.hasFresh(url);
export async function loadOutline(url: string, signal: AbortSignal) {
  try {
    return await outlines.get(url, signal, -10);
  } catch (error) {
    signal.throwIfAborted();
    if (error instanceof MediaFailure) return null;
    throw error;
  }
}
