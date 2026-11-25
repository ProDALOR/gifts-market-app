import { selectionAt, selectionHref } from "../market-selection";
import { SharedIcon } from "../ui/icons";
import "./navigation.css";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { ReactNode, Dispatch, SetStateAction } from "react";
import type { TelegramContext } from "../telegram";
import { useTelegramBack } from "../telegram-ui";
import { initialLocation, pathRoute } from "./routes";
import type { Page, Route } from "./routes";
export type { Page } from "./routes";
type Memory = Map<string, unknown>;
type Navigation = {
  route: Route;
  currency: string;
  setCurrency: (value: string) => void;
  go: (page: Page, user?: number, replace?: boolean) => void;
  href: (page: Page) => string;
  memory: Memory;
  rememberLocation: () => void;
  registerBack: (close: () => void, priority: number) => () => void;
};
const Context = createContext<Navigation | null>(null);
export const useNavigation = () => useContext(Context);
export function NavigationProvider({
  context,
  children,
}: {
  context: TelegramContext;
  children: (route: Route) => ReactNode;
}) {
  const initial = useRef<ReturnType<typeof initialLocation> | undefined>(
    undefined,
  );
  if (!initial.current) {
    initial.current = initialLocation(
      context,
      new URL(location.href),
      history.state,
    );
    history.replaceState(
      { ...history.state, giftsMarket: initial.current.route },
      "",
      initial.current.href,
    );
  }
  const [route, setRoute] = useState(initial.current.route),
    [currency, setCurrency] = useState(initial.current.currency);
  const memory = useRef<Memory>(new Map()).current;
  const urls = useRef<Record<Page, string>>({
    market: "/",
    portfolio: "/portfolio",
  });
  const scrolls = useRef(new Map<number, number>());
  const pageScrolls = useRef(new Map<string, number>());
  const pageKey = (value: Route) => `${value.page}:${value.user ?? ""}`;
  const current = useRef(route);
  current.current = route;
  const backs = useRef<{ close: () => void; priority: number }[]>([]);
  const [, setBackVersion] = useState(0);
  const registerBack = useCallback((close: () => void, priority: number) => {
    const entry = { close, priority };
    backs.current.push(entry);
    setBackVersion((v) => v + 1);
    return () => {
      backs.current = backs.current.filter((v) => v !== entry);
      setBackVersion((v) => v + 1);
    };
  }, []);
  useTelegramBack(context, backs.current.length > 0 || route.index > 0, () => {
    const top = backs.current.reduce<
      (typeof backs.current)[number] | undefined
    >(
      (best, v) => (!best || v.priority >= best.priority ? v : best),
      undefined,
    );
    if (top) top.close();
    else if (route.index > 0) history.back();
  });
  const [locationVersion, setLocationVersion] = useState(0);
  const rememberLocation = useCallback(() => {
    const next = location.pathname + location.search + location.hash;
    if (urls.current[current.current.page] !== next) {
      urls.current[current.current.page] = next;
      setLocationVersion((v) => v + 1);
    }
  }, []);
  const rememberScroll = useCallback(() => {
    scrolls.current.set(current.current.index, scrollY);
    pageScrolls.current.delete(pageKey(current.current));
    pageScrolls.current.set(pageKey(current.current), scrollY);
    while (scrolls.current.size > 128)
      scrolls.current.delete(scrolls.current.keys().next().value!);
    while (pageScrolls.current.size > 4)
      pageScrolls.current.delete(pageScrolls.current.keys().next().value!);
  }, []);
  const go = useCallback(
    (page: Page, user?: number, replace = false) => {
      urls.current[current.current.page] =
        location.pathname + location.search + location.hash;
      rememberScroll();
      const url = new URL(
        user != null ? `/portfolio/${user}` : urls.current[page],
        location.origin,
      );
      url.searchParams.set("currency", currency);
      const next = {
        ...pathRoute(url.pathname),
        index: replace ? current.current.index : current.current.index + 1,
      };
      if (
        page === current.current.page &&
        next.user === current.current.user &&
        !replace
      )
        return;
      if (!replace)
        scrolls.current.set(
          next.index,
          pageScrolls.current.get(pageKey(next)) ?? 0,
        );
      (replace ? history.replaceState : history.pushState).call(
        history,
        { ...history.state, giftsMarket: next },
        "",
        selectionHref(url.pathname, selectionAt(url)),
      );
      current.current = next;
      setRoute(next);
    },
    [currency, rememberScroll],
  );
  useEffect(() => {
    const previous = history.scrollRestoration;
    history.scrollRestoration = "manual";
    const pop = () => {
      rememberScroll();
      const saved = history.state?.giftsMarket;
      const next = {
        ...pathRoute(location.pathname),
        index:
          Number.isSafeInteger(saved?.index) && saved.index >= 0
            ? saved.index
            : 0,
      };
      const nextCurrency = selectionAt(location).get("currency");
      if (nextCurrency) setCurrency(nextCurrency);
      current.current = next;
      setRoute(next);
    };
    window.addEventListener("popstate", pop);
    return () => {
      history.scrollRestoration = previous;
      window.removeEventListener("popstate", pop);
    };
  }, []);
  useLayoutEffect(() => {
    const target = scrolls.current.get(route.index) ?? 0;
    let done = false;
    const stop = () => {
      done = true;
      observer.disconnect();
      clearTimeout(deadline);
      window.removeEventListener("wheel", stop);
      window.removeEventListener("touchstart", stop);
      window.removeEventListener("keydown", key);
    };
    const key = (event: KeyboardEvent) => {
      if (
        [
          "ArrowDown",
          "ArrowUp",
          "PageDown",
          "PageUp",
          "Home",
          "End",
          " ",
        ].includes(event.key)
      )
        stop();
    };
    const restore = () => {
      if (done) return;
      window.scrollTo(0, target);
      if (Math.abs(scrollY - target) < 2) stop();
    };
    const observer = new ResizeObserver(restore);
    observer.observe(document.body);
    const deadline = setTimeout(stop, 5000);
    window.addEventListener("wheel", stop, { passive: true });
    window.addEventListener("touchstart", stop, { passive: true });
    window.addEventListener("keydown", key);
    const frame = requestAnimationFrame(restore);
    return () => {
      cancelAnimationFrame(frame);
      stop();
    };
  }, [route.index, route.page, route.user]);
  const href = useCallback(
    (page: Page) => {
      const url = new URL(urls.current[page], location.origin);
      url.searchParams.set("currency", currency);
      // Modified clicks/new tabs need an explicit destination without history state.
      if (page === "market") url.searchParams.set("page", "market");
      return selectionHref(url.pathname, selectionAt(url));
    },
    [currency, route, locationVersion],
  );
  const value = useMemo(
    () => ({
      route,
      currency,
      setCurrency,
      go,
      href,
      memory,
      rememberLocation,
      registerBack,
    }),
    [route, currency, go, href, memory, rememberLocation, registerBack],
  );
  return <Context.Provider value={value}>{children(route)}</Context.Provider>;
}
/** Bounded per-shell view state. Leaving a page releases its effects, not its filters/cache. */
export function usePageSlot<T>(key: string, create: () => T): { current: T } {
  const nav = useNavigation();
  const local = useRef<{ current: T } | undefined>(undefined);
  if (nav) {
    const existing = nav.memory.get(key) as { current: T } | undefined;
    if (existing) {
      nav.memory.delete(key);
      nav.memory.set(key, existing);
      return existing;
    }
    const value = { current: create() };
    if (nav.memory.size >= 16)
      nav.memory.delete(nav.memory.keys().next().value!);
    nav.memory.set(key, value);
    return value;
  }
  if (!local.current) local.current = { current: create() };
  return local.current;
}
export function usePageState<T>(
  key: string,
  create: () => T,
): [T, Dispatch<SetStateAction<T>>] {
  const slot = usePageSlot(key, create);
  const [state, setState] = useState(slot.current);
  useLayoutEffect(() => {
    slot.current = state;
  }, [slot, state]);
  return [state, setState];
}
export function useSharedCurrency(initial = "GRAM") {
  const nav = useNavigation();
  const [local, setLocal] = useState(initial);
  return [nav?.currency ?? local, nav?.setCurrency ?? setLocal] as const;
}
export function useNavigationBack(
  enabled: boolean,
  close: () => void,
  priority = 10,
) {
  const nav = useNavigation();
  const latest = useRef(close);
  latest.current = close;
  useLayoutEffect(
    () =>
      enabled ? nav?.registerBack(() => latest.current(), priority) : undefined,
    [enabled, nav?.registerBack, priority],
  );
}
export function AppLink({
  page,
  children,
  onClick,
  ...props
}: Omit<import("react").AnchorHTMLAttributes<HTMLAnchorElement>, "href"> & {
  page: Page;
}) {
  const nav = useNavigation();
  return (
    <a
      {...props}
      href={
        nav?.href(page) ?? (page === "market" ? "/?page=market" : "/portfolio")
      }
      onClick={(event) => {
        onClick?.(event);
        if (
          event.defaultPrevented ||
          !nav ||
          event.button !== 0 ||
          event.metaKey ||
          event.ctrlKey ||
          event.shiftKey ||
          event.altKey ||
          (props.target && props.target !== "_self") ||
          props.download != null
        )
          return;
        event.preventDefault();
        nav.go(page);
      }}
    >
      {children}
    </a>
  );
}
export function AppNavigation({ page }: { page: Page }) {
  const [keyboard, setKeyboard] = useState(false);
  useEffect(() => {
    let baseline = innerHeight,
      width = innerWidth;
    const update = () => {
      if (width !== innerWidth) {
        width = innerWidth;
        baseline = innerHeight;
      }
      baseline = Math.max(baseline, innerHeight);
      const focused =
        document.activeElement instanceof HTMLInputElement ||
        document.activeElement instanceof HTMLTextAreaElement;
      setKeyboard(
        focused &&
          baseline - (window.visualViewport?.height ?? innerHeight) > 120,
      );
    };
    document.addEventListener("focusin", update);
    document.addEventListener("focusout", update);
    window.addEventListener("resize", update);
    window.visualViewport?.addEventListener("resize", update);
    return () => {
      document.removeEventListener("focusin", update);
      document.removeEventListener("focusout", update);
      window.removeEventListener("resize", update);
      window.visualViewport?.removeEventListener("resize", update);
    };
  }, []);
  return (
    <nav
      className={`app-navigation${keyboard ? " keyboard-open" : ""}`}
      aria-label="App navigation"
    >
      {(["market", "portfolio"] as const).map((item) => (
        <AppLink
          page={item}
          key={item}
          className={page === item ? "active" : ""}
          aria-current={page === item ? "page" : undefined}
        >
          <SharedIcon name={item === "market" ? "market" : "portfolio"} />
          <span>{item === "market" ? "Market" : "Portfolio"}</span>
        </AppLink>
      ))}
    </nav>
  );
}
