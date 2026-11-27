import { useEffect, useState } from "react";

export function PortfolioRefreshButton({
  busy,
  ready,
  after,
  refresh,
}: {
  busy: boolean;
  ready: boolean;
  after?: string | null;
  refresh: () => void;
}) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const deadline = after ? Date.parse(after) : 0;
    const tick = () => {
      clearTimeout(timer);
      const current = Date.now();
      setNow(current);
      if (!document.hidden && deadline > current)
        timer = setTimeout(tick, Math.min(1000, deadline - current));
    };
    tick();
    document.addEventListener("visibilitychange", tick);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", tick);
    };
  }, [after]);
  const cooldown = after
    ? Math.max(0, Math.ceil((Date.parse(after) - now) / 1000))
    : 0;
  const remaining = `${Math.floor(cooldown / 60)}:${String(cooldown % 60).padStart(2, "0")}`;
  const status = busy
    ? "Checking gifts"
    : !ready
      ? "Waiting for connection"
      : cooldown
        ? `Recheck available in ${remaining}`
        : "Recheck Telegram gifts";
  return (
    <button
      className={`portfolio-refresh${busy ? " busy" : ""}`}
      disabled={!ready || busy || cooldown > 0}
      onClick={refresh}
      aria-label="Refresh gifts"
      aria-description={status}
      aria-busy={busy}
      title={status}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M20 7v5h-5M4 17v-5h5M20 12a8 8 0 0 0-14-5L4 9M4 12a8 8 0 0 0 14 5l2-2" />
      </svg>
      {cooldown > 0 && !busy && (
        <span className="portfolio-cooldown" aria-hidden="true">
          {remaining}
        </span>
      )}
    </button>
  );
}
