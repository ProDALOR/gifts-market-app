import { useEffect, useRef, useState } from "react";
import { animationRenderer, loadAnimationData } from "./animation-data";
import { AnimationLifecycle } from "./animation-lifecycle";
import { retryableMedia, retryMedia } from "./media-resource";
import { playbackBudget } from "./playback-budget";
import type { PlaybackGroup, PlaybackLease } from "./playback-budget";
import { observePreviewSize } from "./preview-environment";
export default function ModelAnimation({
  url,
  active,
  retain,
  priority,
  group,
  vector = false,
  ready,
  onReady,
}: {
  url: string;
  active: boolean;
  retain: boolean;
  priority: number;
  group: PlaybackGroup;
  vector?: boolean;
  ready: boolean;
  onReady: (ready: boolean) => void;
}) {
  const container = useRef<HTMLSpanElement>(null);
  const lifecycle = useRef<AnimationLifecycle | null>(null);
  const lease = useRef<PlaybackLease | undefined>(undefined);
  const readyRef = useRef(ready);
  readyRef.current = ready;
  const [failure, setFailure] = useState<{ error: unknown }>();
  const priorityRef = useRef(priority);
  priorityRef.current = priority;
  useEffect(() => setFailure(undefined), [active, url, vector]);
  useEffect(() => {
    if (!active || !failure || !retryableMedia(failure.error)) return;
    const stop = new AbortController();
    // The failed player has released its budget slot. Recover data before
    // requesting another player; waiting never blocks healthy autoplay.
    void retryMedia(
      () => loadAnimationData(url, stop.signal, priorityRef.current),
      stop.signal,
    ).then(
      () => {
        if (!stop.signal.aborted) setFailure(undefined);
      },
      () => {
        /* Invalid data stays on its still image until reactivation. */
      },
    );
    return () => stop.abort();
  }, [active, failure, url, vector]);
  useEffect(() => {
    const host = container.current;
    if (!host) return;
    const controller = new AnimationLifecycle(
      async (signal) => {
        // A React cleanup can release several slots together. Let the commit finish
        // before starting downloads for transient grants to other closing models.
        await Promise.resolve();
        signal.throwIfAborted();
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
        // CSS can resize artwork without a window resize (search, selected
        // traits, sidebar reflow). Resize the existing canvas, not its player.
        const unobserve =
          renderer === "canvas"
            ? observePreviewSize(host, () => animation.resize())
            : undefined;
        return {
          play: () => animation.play(),
          pause: () => animation.pause(),
          destroy: () => {
            unobserve?.();
            animation.destroy();
          },
          ready: () => animation.isLoaded,
          listen: (event, callback) => {
            animation.addEventListener(event, callback);
          },
        };
      },
      (value) => {
        readyRef.current = value;
        if (value) lease.current?.ready();
        onReady(value);
      },
      350,
      (error) => setFailure({ error }),
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
    if (!active || failure) {
      controller.setActive(false, failure ? false : retain);
      return;
    }
    const current = playbackBudget.request(
      { priority, group },
      (granted, preempted) =>
        controller.setActive(granted, preempted ? false : retain),
    );
    lease.current = current;
    // A retained renderer can resume synchronously before request() returns.
    if (readyRef.current) current.ready();
    return () => {
      lease.current = undefined;
      current.cancel();
    };
  }, [url, active, retain, priority, group, failure, vector, onReady]);
  return (
    <span
      ref={container}
      className={`model-animation${ready ? " ready" : ""}`}
      aria-hidden="true"
    />
  );
}
