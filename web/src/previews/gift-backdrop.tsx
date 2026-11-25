import { useEffect, useRef } from "react";
import type { BackdropPalette } from "./gift-appearance";

import { bannerPatternTiles, giftPatternTiles } from "./preview-layout";
import { loadPattern } from "./pattern-data";
import { observePreviewSize } from "./preview-environment";

/** Paint a still backdrop once per size/palette change, never on animation frames. */
export function GiftBackdrop({
  palette,
  patternUrls,
  visible,
  layout = "gift",
  background = true,
  symbolScale = 1,
}: {
  palette: BackdropPalette;
  patternUrls?: readonly string[];
  visible: boolean;
  layout?: "gift" | "card" | "banner";
  background?: boolean;
  symbolScale?: number;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const { center, edge, pattern } = palette;
  const urls = patternUrls || [];
  const key = JSON.stringify([urls, pattern]);
  useEffect(() => {
    const target = canvas.current;
    if (!target) return;
    if (!visible) {
      target.width = target.height = 1;
      return;
    }
    const stop = new AbortController();
    const symbols: (HTMLCanvasElement | undefined)[] = new Array(urls.length);
    let frame = 0;
    const draw = () => {
      const width = target.clientWidth,
        height = target.clientHeight;
      if (!width || !height) return;
      const dpr = Math.min(devicePixelRatio || 1, 2);
      target.width = Math.round(width * dpr);
      target.height = Math.round(height * dpr);
      const context = target.getContext("2d");
      if (!context) return;
      context.scale(dpr, dpr);
      if (background) {
        const gradient = context.createRadialGradient(
          width / 2,
          height * 0.42,
          0,
          width / 2,
          height * 0.42,
          Math.max(width, height) * 0.72,
        );
        gradient.addColorStop(0, center);
        gradient.addColorStop(1, edge);
        context.fillStyle = gradient;
        context.fillRect(0, 0, width, height);
      }
      if (!symbols.some(Boolean)) return;
      context.globalAlpha = 0.3;
      if (layout === "banner") {
        for (const tile of bannerPatternTiles(
          width,
          height,
          symbols.length,
          symbolScale,
        )) {
          const symbol = symbols[tile.symbol];
          if (symbol)
            context.drawImage(symbol, tile.x, tile.y, tile.size, tile.size);
        }
      } else {
        const symbol = symbols[0];
        if (symbol)
          for (const tile of giftPatternTiles(
            width,
            height,
            layout === "card",
          )) {
            context.globalAlpha = tile.opacity;
            context.drawImage(symbol, tile.x, tile.y, tile.size, tile.size);
          }
      }
      context.globalAlpha = 1;
    };
    // Data completions and resize notifications share one paint per frame.
    const schedule = () => {
      if (!frame)
        frame = requestAnimationFrame(() => {
          frame = 0;
          draw();
        });
    };
    const unobserve = observePreviewSize(target, schedule);
    schedule();
    urls.forEach((url, index) => {
      // Shared resources bound HTTP/decode work; one slow symbol doesn't block others.
      void loadPattern(url, pattern, stop.signal).then(
        (symbol) => {
          if (stop.signal.aborted) return;
          symbols[index] = symbol;
          schedule();
        },
        () => {
          /* Keep the gradient for unavailable symbols. */
        },
      );
    });
    return () => {
      stop.abort();
      unobserve();
      cancelAnimationFrame(frame);
    };
  }, [center, edge, key, layout, background, symbolScale, visible]);
  return <canvas ref={canvas} className="gift-backdrop" aria-hidden="true" />;
}
