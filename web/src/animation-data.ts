import { MediaStore, fetchMedia } from "./media-client.ts";
import { Gunzip } from "fflate";

const MAX_TGS_BYTES = 256 * 1024;
const MAX_JSON_BYTES = 2 * 1024 * 1024;
export type AnimationData = Record<string, unknown>;
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
const animations = new MediaStore<AnimationData>(async (url, signal) => {
  const data = decodeTgs(await fetchMedia(url, signal, "tgs"));
  return {
    value: data,
    bytes: new TextEncoder().encode(JSON.stringify(data)).length * 2,
  };
});
export const loadAnimationData = (url: string, signal: AbortSignal) =>
  animations.get(url, signal);
