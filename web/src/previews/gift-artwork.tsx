import {
  lazy,
  memo,
  Suspense,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import type { GiftMedia } from "../market";
import type { ReactNode } from "react";
import type { BackdropPalette } from "./gift-appearance";
import { GiftPlaceholder } from "./gift-placeholder";
import { GiftBackdrop } from "./gift-backdrop";
import {
  usePreviewEnvironment,
  usePreviewVisibility,
} from "./preview-environment";
import { recoverImage } from "./media-client";
import { useSettledVisibility } from "./use-playback";
import type { PlaybackPolicy } from "./use-playback";
const ModelAnimation = lazy(() => import("./model-animation"));

type ArtworkProps = {
  media?: GiftMedia;
  palette?: BackdropPalette;
  patternUrl?: string;
  label?: string;
  fallback: string;
  size?: "small" | "large";
  playback?: PlaybackPolicy;
  className?: string;
  children?: ReactNode;
};

/** Media identity owns thumbnail failures, hover state and renderer lifetime for every caller. */
export const GiftArtwork = memo(function GiftArtwork(props: ArtworkProps) {
  return (
    <Artwork
      key={JSON.stringify([
        props.media?.thumbnail_url,
        props.media?.animation_url,
        props.size === "large" || props.playback === "detail"
          ? props.media?.preview_url
          : undefined,
      ])}
      {...props}
    />
  );
});

function Artwork({
  media,
  palette,
  patternUrl,
  label = "Gift preview",
  fallback,
  size = "small",
  playback = "interaction",
  className = "",
  children,
}: ArtworkProps) {
  const highResolution = size === "large" || playback === "detail";
  const [previewFailed, setPreviewFailed] = useState(false);
  const preferred = highResolution
    ? (media?.preview_url ?? media?.thumbnail_url)
    : media?.thumbnail_url;
  const url = previewFailed ? media?.thumbnail_url : preferred;
  const animationUrl = media?.animation_url;
  const host = useRef<HTMLSpanElement>(null);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const visible = usePreviewVisibility(host);
  const { pageVisible, reducedMotion } = usePreviewEnvironment();
  const hoverTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const canAnimate =
    !!animationUrl && playback !== "static" && pageVisible && !reducedMotion;
  const canInteract = canAnimate && playback !== "detail";
  const settled = useSettledVisibility(
    visible &&
      canAnimate &&
      (playback === "collection" || playback === "selected"),
    playback === "collection" ? 180 : 100,
  );
  const interactive = hovered || focused;
  const priority =
    playback === "detail"
      ? 100
      : interactive
        ? 30
        : highResolution
          ? 25
          : playback === "selected"
            ? 20
            : 10;
  const shouldAnimate =
    canAnimate &&
    visible &&
    (playback === "detail" ||
      interactive ||
      (settled && playback !== "interaction"));
  useEffect(() => () => clearTimeout(hoverTimer.current), [canInteract]);
  const [attempt, setAttempt] = useState(0);
  const [failed, setFailed] = useState(false);
  // One owner for poster, animation and loading UI; either ready source stops outlines.
  const [ready, setReady] = useState({ still: false, animation: false });
  const setStillReady = useCallback((still: boolean) => {
    setReady((previous) =>
      previous.still === still ? previous : { ...previous, still },
    );
  }, []);
  const setAnimationReady = useCallback((animation: boolean) => {
    setReady((previous) =>
      previous.animation === animation ? previous : { ...previous, animation },
    );
  }, []);
  const artworkReady = ready.still || ready.animation;
  const [requested, setRequested] = useState(false);
  useEffect(() => {
    if (shouldAnimate) setRequested(true);
  }, [shouldAnimate]);
  const recoveryUrl = previewFailed ? preferred : failed ? url : undefined;
  useEffect(() => {
    if (!recoveryUrl || !visible || !pageVisible) return;
    const stop = new AbortController();
    void recoverImage(recoveryUrl, stop.signal).then(
      () => {
        if (stop.signal.aborted) return;
        if (recoveryUrl === preferred) setPreviewFailed(false);
        setFailed(false);
        setStillReady(false);
        setAttempt((value) => value + 1);
      },
      () => {
        /* A known missing/invalid asset retries on the next visible use after its deadline. */
      },
    );
    return () => stop.abort();
  }, [recoveryUrl, preferred, visible, pageVisible, setStillReady]);
  return (
    <span
      ref={host}
      onPointerEnter={
        canInteract
          ? (event) => {
              if (event.pointerType !== "touch") {
                clearTimeout(hoverTimer.current);
                hoverTimer.current = setTimeout(() => setHovered(true), 150);
              }
            }
          : undefined
      }
      onPointerLeave={() => {
        clearTimeout(hoverTimer.current);
        setHovered(false);
      }}
      onFocus={canInteract ? () => setFocused(true) : undefined}
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
          layout={size === "large" ? "preview" : "gift"}
          visible={visible && pageVisible}
        />
      )}
      {!artworkReady &&
        (media ? (
          <GiftPlaceholder
            key={media?.placeholder_url}
            url={media?.placeholder_url}
            active={visible && pageVisible}
            loading={!failed}
          />
        ) : (
          <span className="gift-preview-fallback">{fallback}</span>
        ))}
      {url && !failed && (
        <img
          key={attempt}
          src={url}
          alt=""
          loading={playback === "detail" ? "eager" : "lazy"}
          decoding="async"
          className={`gift-model-image${ready.still ? " loaded" : ""}`}
          onLoad={() => setStillReady(true)}
          onError={() => {
            setStillReady(false);
            if (!previewFailed && preferred !== media?.thumbnail_url) {
              setPreviewFailed(true);
              setAttempt(0);
            } else setFailed(true);
          }}
        />
      )}
      {requested && playback !== "static" && animationUrl && (
        <Suspense fallback={null}>
          <ModelAnimation
            url={animationUrl}
            active={shouldAnimate}
            vector={highResolution}
            priority={priority}
            retain={visible && pageVisible && !reducedMotion}
            ready={ready.animation}
            onReady={setAnimationReady}
          />
        </Suspense>
      )}
      {children}
    </span>
  );
}
