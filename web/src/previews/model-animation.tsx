import { useEffect, useRef, useState } from "react";
import { animationRenderer, loadAnimationData } from "./animation-data";
import { AnimationLifecycle } from "./animation-lifecycle";
import { playbackBudget } from "./playback-budget";
export default function ModelAnimation({
  url,
  active,
  retain,
  priority,
  vector = false,
  ready,
  onReady,
}: {
  url: string;
  active: boolean;
  retain: boolean;
  priority: number;
  vector?: boolean;
  ready: boolean;
  onReady: (ready: boolean) => void;
}) {
  const container = useRef<HTMLSpanElement>(null);
  const lifecycle = useRef<AnimationLifecycle | null>(null);
  const [failed, setFailed] = useState(false);
  const priorityRef = useRef(priority);
  priorityRef.current = priority;
  useEffect(() => setFailed(false), [active, url, vector]);
  useEffect(() => {
    const host = container.current;
    if (!host) return;
    const controller = new AnimationLifecycle(
      async (signal) => {
        const data = await loadAnimationData(url, signal, priorityRef.current);
        const renderer = vector ? "svg" : animationRenderer(data);
        signal.throwIfAborted();
        const player = await (renderer === "svg"
          ? import("lottie-web/build/player/lottie_light")
          : import("lottie-web/build/player/lottie_light_canvas"));
        signal.throwIfAborted();
        // The two lazy bundles have independent ID generators for masks and gradients.
        player.default.setIDPrefix(`gift-${renderer}-`);
        const animation = player.default.loadAnimation({
          container: host,
          renderer,
          loop: true,
          autoplay: false,
          animationData: structuredClone(data),
          rendererSettings:
            renderer === "canvas"
              ? {
                  dpr: Math.min(window.devicePixelRatio || 1, 2),
                  clearCanvas: true,
                }
              : {},
        });
        return {
          play: () => animation.play(),
          pause: () => animation.pause(),
          destroy: () => animation.destroy(),
          ready: () => animation.isLoaded,
          listen: (event, callback) => {
            animation.addEventListener(event, callback);
          },
        };
      },
      onReady,
      350,
      () => setFailed(true),
    );
    lifecycle.current = controller;
    return () => {
      controller.stop();
      lifecycle.current = null;
    };
  }, [url, vector, onReady]);
  useEffect(() => {
    const controller = lifecycle.current;
    if (!controller) return;
    if (!active || failed) {
      controller.setActive(false, failed ? false : retain);
      return;
    }
    return playbackBudget.request(priority, (granted, preempted) =>
      controller.setActive(granted, preempted ? false : retain),
    );
  }, [url, active, retain, priority, failed, vector, onReady]);
  return (
    <span
      ref={container}
      className={`model-animation${ready ? " ready" : ""}`}
      aria-hidden="true"
    />
  );
}
