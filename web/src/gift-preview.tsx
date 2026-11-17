import {
  lazy,
  memo,
  Suspense,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { ReactNode } from "react";
import type { Order } from "./market";
import { giftAppearance } from "./gift-appearance";
import type { GiftOption, BackdropPalette } from "./gift-appearance";
import { GiftBackdrop } from "./gift-backdrop";

import {
  usePreviewEnvironment,
  usePreviewVisibility,
} from "./preview-environment";
import { recoverImage } from "./media-client";

const ModelAnimation = lazy(() => import("./model-animation"));

export function GiftPreview({
  order,
  fallback,
  options,
  large = false,
  autoplay = false,
  className = "",
  children,
}: {
  order: Pick<Order, "attributes">;
  options: ReadonlyMap<string, GiftOption>;
  fallback: string;
  large?: boolean;
  autoplay?: boolean;
  className?: string;
  children?: ReactNode;
}) {
  const appearance = giftAppearance(order, options);
  const url = appearance.modelUrl;
  const palette = useMemo(
    () => appearance.palette,
    [
      appearance.palette?.center,
      appearance.palette?.edge,
      appearance.palette?.pattern,
    ],
  );
  return (
    <Preview
      key={url || "none"}
      url={url}
      palette={palette}
      patternUrl={appearance.patternUrl}
      label={appearance.label}
      fallback={fallback}
      large={large}
      autoplay={autoplay}
      className={className}
      children={children}
    />
  );
}
const Preview = memo(function Preview({
  palette,
  patternUrl,
  label,
  url,
  fallback,
  large,
  autoplay,
  className,
  children,
}: {
  url?: string;
  palette?: BackdropPalette;
  patternUrl?: string;
  label: string;
  fallback: string;
  large: boolean;
  autoplay: boolean;
  className: string;
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
  const shouldAnimate =
    !!url &&
    visible &&
    pageVisible &&
    !reducedMotion &&
    (large || autoplay || hovered || focused);
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
      tabIndex={url && !large && !autoplay && !children ? 0 : undefined}
      role={children ? "group" : "img"}
      aria-label={label}
      className={`item-icon gift-preview${large ? " gift-preview-large" : ""}${palette ? " gift-preview-decorated" : ""} ${className}`}
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
          loading={large ? "eager" : "lazy"}
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
            retain={visible && pageVisible && !reducedMotion}
          />
        </Suspense>
      )}
      {children}
    </span>
  );
});
