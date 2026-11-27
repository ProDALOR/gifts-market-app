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
import { useArtworkImage } from "./use-artwork-image";
import { useSettledVisibility } from "./use-playback";
import type { PlaybackPolicy } from "./use-playback";
import { useDisplayMode } from "../app/preferences";
import { artworkPolicy, type ArtworkSurface } from "./artwork-policy";
const ModelAnimation = lazy(() => import("./model-animation"));

type ArtworkProps = {
  media?: GiftMedia;
  palette?: BackdropPalette;
  patternUrls?: readonly string[];
  patternLayout?: "gift" | "card";
  label?: string;
  fallback: string;
  size?: "small" | "large";
  playback?: PlaybackPolicy;
  surface?: ArtworkSurface;
  /** Load stills and symbols ahead of visibility inside a bounded virtual grid. */
  preload?: boolean;
  className?: string;
  placeholderKind?: "gift" | "symbol";
  children?: ReactNode;
};

/** Media identity owns image failures, hover state and renderer lifetime for every caller. */
export const GiftArtwork = memo(function GiftArtwork(props: ArtworkProps) {
  const mode = useDisplayMode();
  const policy = artworkPolicy(
    mode,
    props.surface ?? "preview",
    props.playback ?? "interaction",
  );
  return (
    <Artwork
      key={JSON.stringify([
        props.media?.thumbnail_url,
        props.media?.animation_url,
        props.media?.preview_url,
        policy.format,
        policy.playback,
      ])}
      {...props}
      {...policy}
    />
  );
});

function Artwork({
  media,
  palette,
  patternUrls,
  patternLayout = "gift",
  label = "Gift preview",
  fallback,
  size = "small",
  playback = "interaction",
  preload = false,
  className = "",
  placeholderKind = "gift",
  children,
  surface,
  format,
}: ArtworkProps & { format: "thumbnail" | "preview" | "animation" }) {
  const preferVectorPlayback = size === "large" || playback === "detail";
  const preferred =
    format === "animation"
      ? undefined
      : format === "thumbnail"
        ? media?.thumbnail_url
        : (media?.preview_url ?? media?.thumbnail_url);
  const animationUrl = media?.animation_url;
  const host = useRef<HTMLSpanElement>(null);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const visible = usePreviewVisibility(host);
  const nearby = usePreviewVisibility(host, 256, preload);
  const { pageVisible, reducedMotion } = usePreviewEnvironment();
  const image = useArtworkImage(
    preferred,
    format === "animation" ? undefined : media?.thumbnail_url,
    visible && pageVisible,
  );
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
  const group =
    playback === "detail"
      ? "detail"
      : interactive
        ? "interaction"
        : playback === "collection"
          ? "collection"
          : "selection";
  const priority =
    group === "detail"
      ? 100
      : group === "interaction"
        ? 30
        : preferVectorPlayback
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
  useEffect(() => {
    clearTimeout(hoverTimer.current);
    if (!visible || !canInteract) {
      setHovered(false);
    } else if (host.current?.matches(":hover")) {
      hoverTimer.current = setTimeout(() => setHovered(true), 150);
    }
    return () => clearTimeout(hoverTimer.current);
  }, [visible, canInteract]);
  const [animationReady, setAnimationReady] = useState(false);
  const [animationFailed, setAnimationFailed] = useState(false);
  const artworkReady = image.ready || animationReady;
  const [requested, setRequested] = useState(false);
  useEffect(() => {
    if (shouldAnimate) setRequested(true);
  }, [shouldAnimate]);
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
        (preferred || animationUrl) &&
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
          patternUrls={patternUrls}
          layout={patternLayout}
          visible={(visible || nearby) && pageVisible}
          preload={preload && pageVisible}
        />
      )}
      {!artworkReady &&
        (format === "animation" ||
        surface === "item" ||
        media ||
        placeholderKind === "symbol" ? (
          <GiftPlaceholder
            kind={placeholderKind}
            active={visible && pageVisible}
            loading={image.loading || (shouldAnimate && !animationFailed)}
          />
        ) : (
          <span className="gift-preview-fallback">{fallback}</span>
        ))}
      {image.sources.map((source) => (
        <ArtworkImage
          key={`${source.url}:${source.attempt}`}
          source={source}
          eager={playback === "detail" || preload}
          priority={visible ? "high" : "auto"}
        />
      ))}
      {requested && playback !== "static" && animationUrl && (
        <Suspense fallback={null}>
          <ModelAnimation
            url={animationUrl}
            active={shouldAnimate}
            vector={preferVectorPlayback}
            priority={priority}
            group={group}
            retain={visible && pageVisible && !reducedMotion}
            ready={animationReady}
            onReady={setAnimationReady}
            onFailed={setAnimationFailed}
          />
        </Suspense>
      )}
      {children}
    </span>
  );
}

function ArtworkImage({
  source,
  eager,
  priority,
}: {
  source: ReturnType<typeof useArtworkImage>["sources"][number];
  eager: boolean;
  priority: "high" | "auto";
}) {
  const complete = useCallback(
    (image: HTMLImageElement | null) => {
      // Memory-cache completions can precede React's load listener. Reveal before the first paint.
      if (image?.complete && image.naturalWidth) source.loaded(image);
    },
    [source.loaded],
  );
  return (
    <img
      ref={complete}
      src={source.url}
      alt=""
      style={{ visibility: source.status !== "ready" ? "hidden" : undefined }}
      loading={eager ? "eager" : "lazy"}
      fetchPriority={priority}
      decoding="async"
      className={`gift-model-image${source.status === "ready" ? " loaded" : ""}`}
      onLoad={(event) => source.loaded(event.currentTarget)}
      onError={source.failed}
    />
  );
}
