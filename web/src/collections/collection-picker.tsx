import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { GiftArtwork } from "../previews/gift-artwork";
import {
  observePreviewSize,
  usePreviewEnvironment,
} from "../previews/preview-environment";
import { SearchIcon } from "../ui/icons";
import type { Collection } from "../orderbook";
import { CollectionFloor, useFloorReport } from "../valuation/floors";
export const CollectionRail = memo(function CollectionRail({
  collections,
  selected,
  select,
}: {
  collections: Collection[];
  selected: string;
  select: (id: string) => void;
}) {
  const { visible } = useFloorReport();
  const [searching, setSearching] = useState(false),
    [query, setQuery] = useState("");
  const { reducedMotion } = usePreviewEnvironment();
  const initial = useRef(true);
  const rail = useRef<HTMLDivElement>(null),
    host = useRef<HTMLElement>(null),
    input = useRef<HTMLInputElement>(null),
    selectedButton = useRef<HTMLButtonElement>(null);
  const matches = collections.filter((item) =>
    item.name.toLowerCase().includes(query.trim().toLowerCase()),
  );
  useLayoutEffect(() => {
    const label = selectedButton.current?.querySelector("b");
    if (!label || searching) return;
    const context = document.createElement("canvas").getContext("2d");
    if (!context) return;
    const fit = () => {
      const styles = getComputedStyle(label);
      if (styles.whiteSpace === "nowrap") return;
      const words = (label.textContent || "").trim().split(/\s+/);
      const width = label.clientWidth;
      let fitted = 11;
      for (let size = 16; size >= 11; size -= 0.25) {
        context.font = `${styles.fontWeight} ${size}px ${styles.fontFamily}`;
        let line = "",
          lines = 1,
          fits = true;
        for (const word of words) {
          if (context.measureText(word).width > width) {
            fits = false;
            break;
          }
          const next = line ? `${line} ${word}` : word;
          if (context.measureText(next).width <= width) line = next;
          else {
            lines++;
            line = word;
          }
        }
        fitted = size;
        if (fits && lines <= 2) break;
      }
      label.style.setProperty("--collection-name-size", `${fitted}px`);
    };
    fit();
    return observePreviewSize(label, fit);
  }, [selected, searching, collections]);
  const alignSelected = useCallback(
    (instant = false) => {
      if (rail.current && selectedButton.current) {
        const styles = getComputedStyle(rail.current);
        if (styles.flexDirection === "column") {
          const viewport = rail.current.getBoundingClientRect();
          const selected = selectedButton.current.getBoundingClientRect();
          const offset =
            selected.top < viewport.top
              ? selected.top - viewport.top
              : selected.bottom > viewport.bottom
                ? selected.bottom - viewport.bottom
                : 0;
          rail.current.scrollTo({
            left: 0,
            top: rail.current.scrollTop + offset,
            behavior: reducedMotion || instant ? "instant" : "smooth",
          });
          return;
        }
        const precedingSlot =
          parseFloat(styles.getPropertyValue("--collection-tile-size")) +
          parseFloat(styles.columnGap);
        const left = Math.max(
          0,
          selectedButton.current.getBoundingClientRect().left -
            rail.current.getBoundingClientRect().left +
            rail.current.scrollLeft -
            precedingSlot,
        );
        rail.current.scrollTo({
          left,
          behavior: reducedMotion || instant ? "instant" : "smooth",
        });
      }
    },
    [reducedMotion],
  );
  useEffect(() => {
    if (searching) {
      rail.current?.scrollTo({ left: 0, top: 0 });
      input.current?.focus();
      initial.current = true;
    } else {
      alignSelected(initial.current);
      initial.current = false;
    }
  }, [searching, selected, collections.length, alignSelected]);
  useEffect(() => {
    const element = host.current;
    if (!element || searching) return;
    let width = element.clientWidth,
      height = element.clientHeight;
    return observePreviewSize(element, () => {
      if (width === element.clientWidth && height === element.clientHeight)
        return;
      width = element.clientWidth;
      height = element.clientHeight;
      alignSelected(true);
    });
  }, [searching, alignSelected]);
  function close() {
    setSearching(false);
    setQuery("");
  }
  const visibleSignature = JSON.stringify(matches.map((item) => item.id));
  useEffect(() => {
    const element = rail.current;
    if (!element) return;
    const shown = new Set<string>();
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          const id = (entry.target as HTMLElement).dataset.collectionId!;
          if (entry.isIntersecting) shown.add(id);
          else shown.delete(id);
        }
        visible([...shown]);
      },
      { root: element, rootMargin: "80px" },
    );
    element
      .querySelectorAll("[data-collection-id]")
      .forEach((node) => observer.observe(node));
    return () => {
      observer.disconnect();
      visible([]);
    };
  }, [visibleSignature, searching, visible]);
  return (
    <nav
      ref={host}
      className={`collection-rail${searching ? " searching" : ""}`}
      aria-label="Collections"
      onTransitionEnd={(event) => {
        // Width changes move the selected tile; align again at its final position.
        if (
          !searching &&
          event.propertyName === "width" &&
          (event.target as HTMLElement).classList.contains("collection-tile")
        )
          alignSelected();
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape" && searching) {
          close();
          event.currentTarget
            .querySelector<HTMLButtonElement>(".search-toggle")
            ?.focus();
        }
      }}
    >
      <div className={`search-tile${searching ? " open" : ""}`}>
        <button
          className="search-toggle"
          aria-label="Search collections"
          aria-expanded={searching}
          aria-controls="collection-query"
          onClick={() => setSearching(true)}
          tabIndex={searching ? -1 : 0}
        >
          <SearchIcon />
        </button>
        <input
          ref={input}
          id="collection-query"
          type="search"
          hidden={!searching}
          aria-label="Filter collections"
          placeholder="Search"
          autoComplete="off"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (
              event.key === "Enter" &&
              !event.nativeEvent.isComposing &&
              matches[0]
            ) {
              event.preventDefault();
              select(matches[0].id);
              close();
            }
          }}
        />
        {searching && (
          <button
            className="search-close"
            aria-label="Close collection search"
            onClick={close}
          >
            ×
          </button>
        )}
      </div>
      <div className="collection-list" ref={rail}>
        {matches.map((item) => {
          const media = item.preview;
          return (
            <button
              ref={item.id === selected ? selectedButton : undefined}
              className={`collection-tile tile-color-${Array.from(item.id).reduce((hash, char) => hash + char.charCodeAt(0), 0) % 6}`}
              key={item.id}
              data-collection-id={item.id}
              aria-label={item.name}
              aria-pressed={item.id === selected}
              title={item.name}
              onClick={() => {
                select(item.id);
                close();
              }}
            >
              <span className="collection-art">
                {media ? (
                  <GiftArtwork
                    media={media}
                    fallback={item.name[0]}
                    label={item.name}
                    playback="collection"
                  />
                ) : (
                  <span>{item.name[0]}</span>
                )}
                <span className="collection-floor">
                  <CollectionFloor id={item.id} />
                </span>
              </span>
              <b>{item.name}</b>
            </button>
          );
        })}
        {!matches.length && (
          <span className="rail-empty" role="status">
            No collections found
          </span>
        )}
      </div>
    </nav>
  );
});
