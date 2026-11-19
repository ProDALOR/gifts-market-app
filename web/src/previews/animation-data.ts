import { MediaResource } from "./media-resource.ts";
import { fetchMedia } from "./media-client.ts";
import { Gunzip } from "fflate";

const MAX_TGS_BYTES = 256 * 1024;
const MAX_JSON_BYTES = 2 * 1024 * 1024;
export type AnimationData = Record<string, unknown>;
type Renderer = "canvas" | "svg";
const renderers = new WeakMap<AnimationData, Renderer>();

/** Canvas drops gradient opacity when its stop positions differ from the color stops. */
export function animationRenderer(data: AnimationData): Renderer {
  const cached = renderers.get(data);
  if (cached) return cached;
  const pending: unknown[] = [data.layers, data.assets];
  let renderer: Renderer = "canvas";
  while (pending.length) {
    const node = pending.pop();
    if (Array.isArray(node)) {
      for (const child of node) pending.push(child);
    } else if (node && typeof node === "object") {
      const shape = node as Record<string, unknown>;
      if (
        (shape.ty === "gf" || shape.ty === "gs") &&
        needsOpacityMask(shape.g)
      ) {
        renderer = "svg";
        break;
      }
      // Only visit scene structure, not geometry coordinates or every animation keyframe.
      pending.push(shape.layers, shape.shapes, shape.it);
    }
  }
  renderers.set(data, renderer);
  return renderer;
}

function needsOpacityMask(value: unknown): boolean {
  if (!value || typeof value !== "object") return false;
  const gradient = value as { p?: number; k?: { k?: unknown[] } };
  const points = gradient.p;
  const values = gradient.k?.k;
  if (
    !points ||
    !Number.isInteger(points) ||
    points < 1 ||
    !Array.isArray(values)
  )
    return false;
  const separate = (stops: unknown) => {
    if (!Array.isArray(stops) || stops.length <= points * 4) return false;
    if (stops.length - points * 4 !== points * 2) return true;
    for (let index = 0; index < points; index++)
      if (Math.abs(stops[index * 4] - stops[points * 4 + index * 2]) > 0.01)
        return true;
    return false;
  };
  return typeof values[0] === "number"
    ? separate(values)
    : values.some((frame) => {
        if (!frame || typeof frame !== "object") return false;
        const keyframe = frame as { s?: unknown; e?: unknown };
        return separate(keyframe.s) || separate(keyframe.e);
      });
}
export function decodeTgs(bytes: Uint8Array): AnimationData {
  if (bytes.length > MAX_TGS_BYTES) throw new Error("Animation is too large");
  const chunks: Uint8Array[] = [];
  let size = 0;
  const decoder = new Gunzip((chunk) => {
    size += chunk.length;
    if (size > MAX_JSON_BYTES)
      throw new Error("Animation expands beyond the limit");
    chunks.push(chunk);
  });
  decoder.push(bytes, true);
  const json = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    json.set(chunk, offset);
    offset += chunk.length;
  }
  const data = JSON.parse(new TextDecoder().decode(json));
  if (
    !data ||
    typeof data !== "object" ||
    typeof data.v !== "string" ||
    !Array.isArray(data.layers) ||
    !(data.w > 0 && data.w <= 1024 && data.h > 0 && data.h <= 1024) ||
    !(
      data.fr >= 1 &&
      data.fr <= 120 &&
      data.op > data.ip &&
      data.op - data.ip <= data.fr * 10
    )
  ) {
    throw new Error("Invalid animation");
  }
  return data;
}
const animations = new MediaResource<AnimationData>(
  async (url, signal, priority) => {
    const data = decodeTgs(await fetchMedia(url, signal, "tgs", priority));
    return {
      value: data,
      bytes: new TextEncoder().encode(JSON.stringify(data)).length * 2,
    };
  },
);
export const loadAnimationData = (
  url: string,
  signal: AbortSignal,
  priority = 0,
) => animations.get(url, signal, priority);
