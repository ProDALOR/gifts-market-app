import { useEffect, useRef, useState } from "react";
import type { BackdropPalette } from "./gift-appearance";

import { bannerPatternTiles } from "./preview-layout";
import { loadPattern } from "./pattern-data";
import { observePreviewSize } from "./preview-environment";

/** Paint a still backdrop once per size/palette change, never on animation frames. */
export function GiftBackdrop({
  palette,
  patternUrl,
  patternUrls,
  visible,
  layout = "gift",
  background = true,
}: {
  palette: BackdropPalette;
  patternUrl?: string;
  patternUrls?: string[];
  visible: boolean;
  layout?: "gift" | "banner";
  background?: boolean;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const { center, edge, pattern } = palette;
  const urls = patternUrls || (patternUrl ? [patternUrl] : []);
  const key = JSON.stringify([urls, pattern]);
  const [frame, setFrame] = useState<{
    key: string;
    canvases: (HTMLCanvasElement | undefined)[];
  }>();
  const symbols = frame?.key === key ? frame.canvases : [];
  useEffect(() => {
    if (!visible) {
      setFrame(undefined);
      return;
    }
    if (!urls.length) return;
    const stop = new AbortController();
    void (async () => {
      const canvases: (HTMLCanvasElement | undefined)[] = new Array(
        urls.length,
      );
      // Bound work per layer; shared media stores coalesce matching sources/tints.
      for (let index = 0; index < urls.length; index++) {
        try {
          canvases[index] = await loadPattern(
            urls[index],
            pattern,
            stop.signal,
          );
        } catch {
          /* Keep the gradient for unavailable symbols. */
        }
        if (stop.signal.aborted) return;
        setFrame({ key, canvases: canvases.slice() });
      }
    })();
    return () => stop.abort();
  }, [key, visible]);
  useEffect(() => {
    const target = canvas.current;
    if (!target) return;
    if (!visible) {
      target.width = target.height = 1;
      return;
    }
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
      const size = Math.min(width, height) * 0.17;
      context.globalAlpha = 0.3;
      if (layout === "banner") {
        for (const tile of bannerPatternTiles(width, height, symbols.length)) {
          const symbol = symbols[tile.symbol];
          if (symbol)
            context.drawImage(symbol, tile.x, tile.y, tile.size, tile.size);
        }
      } else
        for (let index = 0; index < 8; index++) {
          const angle = -Math.PI / 2 + (index * Math.PI) / 4;
          const x = width * (0.5 + 0.46 * Math.cos(angle));
          const y = height * (0.5 + 0.41 * Math.sin(angle));
          const symbol = symbols[0];
          if (symbol)
            context.drawImage(symbol, x - size / 2, y - size / 2, size, size);
        }
      context.globalAlpha = 1;
    };
    const unobserve = observePreviewSize(target, draw);
    draw();
    return unobserve;
  }, [center, edge, frame, key, layout, background, visible]);
  return <canvas ref={canvas} className="gift-backdrop" aria-hidden="true" />;
}
