import { lazy, memo, Suspense, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import type { BackdropPalette } from "./gift-appearance";
import { GiftBackdrop } from "./gift-backdrop";
import {
  usePreviewEnvironment,
  usePreviewVisibility,
} from "./preview-environment";
import { recoverImage } from "./media-client";
import { useSettledVisibility } from "./use-playback";
import type { PlaybackPolicy } from "./use-playback";
const ModelAnimation = lazy(() => import("./model-animation"));

export const GiftArtwork = memo(function GiftArtwork({
  url,
  palette,
  patternUrl,
  label = "Gift preview",
  fallback,
  size = "small",
  playback = "interaction",
  className = "",
  children,
}: {
  url?: string;
  palette?: BackdropPalette;
  patternUrl?: string;
  label?: string;
  fallback: string;
  size?: "small" | "large";
  playback?: PlaybackPolicy;
  className?: string;
  children?: ReactNode;
}) {
  const host = useRef<HTMLSpanElement>(null);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const visible = usePreviewVisibility(host);
  const { pageVisible, reducedMotion } = usePreviewEnvironment();
  const hoverTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const settled = useSettledVisibility(
    visible,
    playback === "collection" ? 180 : 100,
  );
  const interactive = hovered || focused;
  const priority =
    playback === "detail"
      ? 100
      : interactive
        ? 30
        : playback === "selected"
          ? 20
          : 10;
  const shouldAnimate =
    !!url &&
    visible &&
    pageVisible &&
    !reducedMotion &&
    playback !== "static" &&
    (playback === "detail" ||
      interactive ||
      (settled && playback !== "interaction"));
  useEffect(() => () => clearTimeout(hoverTimer.current), []);
  const [attempt, setAttempt] = useState(0);
  const [failed, setFailed] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [requested, setRequested] = useState(false);
  useEffect(() => {
    if (shouldAnimate) setRequested(true);
  }, [shouldAnimate]);
  useEffect(() => {
    if (!failed || attempt >= 1 || !url || !visible || !pageVisible) return;
    const stop = new AbortController();
    void recoverImage(url, stop.signal)
      .then(() => {
        if (!stop.signal.aborted) {
          setFailed(false);
          setAttempt((value) => value + 1);
        }
      })
      .catch(() => {
        /* Keep the fallback until the next page load. */
      });
    return () => stop.abort();
  }, [failed, attempt, url, visible, pageVisible]);
  return (
    <span
      ref={host}
      onPointerEnter={(event) => {
        if (event.pointerType !== "touch") {
          clearTimeout(hoverTimer.current);
          hoverTimer.current = setTimeout(() => setHovered(true), 150);
        }
      }}
      onPointerLeave={() => {
        clearTimeout(hoverTimer.current);
        setHovered(false);
      }}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      tabIndex={
        url &&
        (playback === "interaction" || playback === "selected") &&
        !children
          ? 0
          : undefined
      }
      role={children ? "group" : "img"}
      aria-label={label}
      className={`item-icon gift-preview${size === "large" ? " gift-preview-large" : ""}${palette ? " gift-preview-decorated" : ""} ${className}`}
    >
      {palette && (
        <GiftBackdrop
          palette={palette}
          patternUrl={patternUrl}
          visible={visible && pageVisible}
        />
      )}
      {!loaded && <span className="gift-preview-fallback">{fallback}</span>}
      {url && !failed && (
        <img
          key={attempt}
          src={url}
          alt=""
          loading={playback === "detail" ? "eager" : "lazy"}
          decoding="async"
          className={`gift-model-image${loaded ? " loaded" : ""}`}
          onLoad={() => setLoaded(true)}
          onError={() => {
            setLoaded(false);
            setFailed(true);
          }}
        />
      )}
      {requested && url && (
        <Suspense fallback={null}>
          <ModelAnimation
            url={`${url}/animation`}
            active={shouldAnimate}
            priority={priority}
            retain={visible && pageVisible && !reducedMotion}
          />
        </Suspense>
      )}
      {children}
    </span>
  );
});
