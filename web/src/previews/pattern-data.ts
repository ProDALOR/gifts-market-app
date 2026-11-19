import { MediaResource } from "./media-resource.ts";
import { fetchMedia } from "./media-client.ts";
// Decode once per pattern URL, then tint once per color. Both caches account for
// decoded RGBA bytes rather than compressed image sizes or an arbitrary count.
const symbols = new MediaResource<HTMLCanvasElement>(
  async (url, signal) => {
    const bytes = await fetchMedia(url, signal, "image");
    const bitmap = await createImageBitmap(new Blob([bytes]), {
      resizeWidth: 96,
      resizeHeight: 96,
    });
    try {
      signal.throwIfAborted();
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 96;
      const context = canvas.getContext("2d");
      if (!context) throw new Error("Canvas is unavailable");
      context.drawImage(bitmap, 0, 0, 96, 96);
      return { value: canvas, bytes: 96 * 96 * 4 };
    } finally {
      bitmap.close();
    }
  },
  2 * 1024 * 1024,
);
const frames = new MediaResource<HTMLCanvasElement>(
  async (key, signal) => {
    const [url, color] = JSON.parse(key) as [string, string];
    const symbol = await symbols.get(url, signal);
    signal.throwIfAborted();
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 96;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Canvas is unavailable");
    context.drawImage(symbol, 0, 0);
    context.globalCompositeOperation = "source-in";
    context.fillStyle = color;
    context.fillRect(0, 0, 96, 96);
    return { value: canvas, bytes: 96 * 96 * 4 };
  },
  2 * 1024 * 1024,
);
export const loadPattern = (url: string, color: string, signal: AbortSignal) =>
  frames.get(JSON.stringify([url, color]), signal);
