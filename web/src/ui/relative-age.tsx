import { useSyncExternalStore } from "react";
import { compactAge } from "./time";

// History cards and portfolio status share one clock. Hidden tabs and empty views stop it.
const listeners = new Set<() => void>();
let now = Date.now();
let timer: ReturnType<typeof setInterval> | undefined;
function tick() {
  now = Date.now();
  listeners.forEach((listener) => listener());
}
function resume() {
  clearInterval(timer);
  timer = undefined;
  if (!document.hidden) {
    tick();
    timer = setInterval(tick, 60_000);
  }
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1) {
    document.addEventListener("visibilitychange", resume);
    resume();
  }
  return () => {
    listeners.delete(listener);
    if (!listeners.size) {
      clearInterval(timer);
      timer = undefined;
      document.removeEventListener("visibilitychange", resume);
    }
  };
}
const snapshot = () => now;

export function RelativeAge({
  at,
  className = "execution-age",
  ago = false,
  label = "Executed",
}: {
  at: string;
  className?: string;
  ago?: boolean;
  label?: string;
}) {
  const current = useSyncExternalStore(subscribe, snapshot);
  const timestamp = Date.parse(at);
  if (!Number.isFinite(timestamp)) return null;
  const age = compactAge(timestamp, current);
  return (
    <time
      className={className}
      dateTime={at}
      aria-label={`${label} ${age} ago`}
    >
      {age}
      {ago && " ago"}
    </time>
  );
}
