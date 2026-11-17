import { useEffect, useRef } from "react";
import { loadAnimationData } from "./animation-data";
import { AnimationLifecycle } from "./animation-lifecycle";
export default function ModelAnimation({
  url,
  active,
  retain,
}: {
  url: string;
  active: boolean;
  retain: boolean;
}) {
  const container = useRef<HTMLSpanElement>(null);
  const lifecycle = useRef<AnimationLifecycle | null>(null);
  useEffect(() => {
    const host = container.current;
    if (!host) return;
    const controller = new AnimationLifecycle(
      async (signal) => {
        const [data, player] = await Promise.all([
          loadAnimationData(url, signal),
          import("lottie-web/build/player/lottie_light_canvas"),
        ]);
        signal.throwIfAborted();
        const animation = player.default.loadAnimation({
          container: host,
          renderer: "canvas",
          loop: true,
          autoplay: false,
          animationData: structuredClone(data),
          rendererSettings: {
            dpr: Math.min(window.devicePixelRatio || 1, 2),
            clearCanvas: true,
          },
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
      (ready) => host.classList.toggle("ready", ready),
    );
    lifecycle.current = controller;
    return () => {
      controller.stop();
      lifecycle.current = null;
    };
  }, [url]);
  useEffect(() => {
    lifecycle.current?.setActive(active, retain);
  }, [url, active, retain]);
  return (
    <span ref={container} className="model-animation" aria-hidden="true" />
  );
}
