import { ApiFailure } from "./api-error.ts";
import type { WebApp, OptionalSchema } from "./telegram.ts";
import { selectionParams, filterLine } from "./market-selection.ts";
import type { MarketSelection } from "./market-selection.ts";

import type { SessionRequest } from "./telegram-session.ts";
export type ShareTarget = OptionalSchema<
  "ShareTarget",
  | { type: "gift"; slug: string; currency_id: string }
  | { type: "market"; selection: MarketSelection }
  | {
      type: "portfolio";
      target: {
        user_id: number;
        currency_id: string;
        filters?: {
          collection_id?: string;
          models?: string[];
          backdrops?: string[];
          patterns?: string[];
          number?: number;
        };
        sort?:
          | "position_asc"
          | "position_desc"
          | "number_asc"
          | "number_desc"
          | "value_asc"
          | "value_desc";
      };
    }
>;
export type SharedMessage = OptionalSchema<
  "SharedMessage",
  {
    prepared?: { id: string; expires_at: number };
    fallback: { type: "inline"; query: string } | { type: "link"; url: string };
  }
>;
export type SharingState = { busy: boolean; error: string };
/** Prepared IDs are optional; share targets carry no report-cache identity or expiry. */
export class SharingController {
  private state: SharingState = { busy: false, error: "" };
  private listeners = new Set<() => void>();
  private pending?: AbortController;
  constructor(
    private app: WebApp | undefined,
    private request: SessionRequest,
    private baseUrl: string | undefined = typeof location === "undefined"
      ? undefined
      : location.origin,
  ) {}
  current = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private publish(state: SharingState) {
    this.state = state;
    for (const listener of this.listeners) listener();
  }
  cancel = () => {
    const old = this.pending;
    this.pending = undefined;
    old?.abort();
    this.publish({ busy: false, error: "" });
  };
  private inline(query: string) {
    if (!this.app?.switchInlineQuery || !query || [...query].length > 256)
      return false;
    try {
      this.app.switchInlineQuery(query, ["users", "groups", "channels"]);
      return true;
    } catch {
      return false;
    }
  }
  private fallback(fallback: SharedMessage["fallback"]) {
    if (fallback.type === "inline") return this.inline(fallback.query);
    if (!this.app?.openTelegramLink) return false;
    try {
      const target = new URL(fallback.url);
      if (
        !["http:", "https:"].includes(target.protocol) ||
        target.username ||
        target.password
      )
        return false;
      const url = new URL("https://t.me/share/url");
      url.searchParams.set("url", target.href);
      this.app.openTelegramLink(url.href);
      return true;
    } catch {
      return false;
    }
  }
  private localTarget(
    target: ShareTarget,
  ): SharedMessage["fallback"] | undefined {
    if (target.type === "gift")
      return { type: "inline", query: `${target.slug} ${target.currency_id}` };
    if (!this.baseUrl) return;
    const url = new URL(this.baseUrl);
    if (target.type === "market")
      url.hash = selectionParams(target.selection).toString();
    else {
      const p = target.target,
        f = p.filters;
      const selection: MarketSelection = {
        filters: {
          collection_id: f?.collection_id,
          currency_id: p.currency_id,
          venue_id: null,
          models: f?.models ?? [],
          backdrops: f?.backdrops ?? [],
          patterns: f?.patterns ?? [],
          number: f?.number == null ? null : { min: f.number, max: f.number },
          price: null,
        },
      };
      url.pathname = `${url.pathname.replace(/\/$/, "")}/portfolio/${p.user_id}`;
      url.hash = new URLSearchParams({
        currency: p.currency_id,
        f: filterLine(selection),
        sort: p.sort ?? "value_desc",
      }).toString();
    }
    return { type: "link", url: url.href };
  }
  private localShare(target: ShareTarget) {
    try {
      const fallback = this.localTarget(target);
      return fallback ? this.fallback(fallback) : false;
    } catch {
      return false;
    }
  }
  share = async (target: ShareTarget) => {
    if (this.pending) return;
    const pending = new AbortController();
    this.pending = pending;
    const current = () => this.pending === pending && !pending.signal.aborted;
    const timer = setTimeout(() => {
      if (current()) {
        this.pending = undefined;
        pending.abort();
        this.publish({
          busy: false,
          error: "Sharing took too long. Please try again.",
        });
      }
    }, 28000);
    this.publish({ busy: true, error: "" });
    let native = false;
    try {
      const result = await this.request<SharedMessage>(
        "/telegram/share",
        target,
        pending.signal,
      );
      if (!current()) return;
      const prepared = result.prepared;
      if (
        prepared?.id &&
        Number.isFinite(prepared.expires_at) &&
        prepared.expires_at * 1000 > Date.now() + 1000 &&
        this.app?.shareMessage &&
        this.app.isVersionAtLeast?.("8.0")
      ) {
        // A false callback includes user cancellation. Never open a second chooser after it.
        try {
          this.app.shareMessage(prepared.id, () => {
            if (current()) {
              this.pending = undefined;
              this.publish({ busy: false, error: "" });
            }
          });
          // The native chooser owns its own lifetime, not the HTTP timeout.
          clearTimeout(timer);
          native = true;
          return;
        } catch {
          // Synchronous unsupported-client failures can safely fall back to inline.
        }
      }
      if (!this.fallback(result.fallback))
        throw new Error("Open or update Telegram to share messages.");
    } catch (error) {
      // Stateless selections remain shareable without prepared support or a signed-in API session.
      if (
        current() &&
        (this.app?.switchInlineQuery || this.app?.openTelegramLink) &&
        (error instanceof TypeError ||
          (error instanceof ApiFailure &&
            ["unavailable", "unauthorized", "not_found", "timed_out"].includes(
              error.code,
            ))) &&
        this.localShare(target)
      ) {
        // A readable target or application link replaces the unavailable native preparation.
      } else if (current())
        this.publish({
          busy: false,
          error:
            error instanceof Error
              ? error.message
              : "Could not prepare this share.",
        });
    } finally {
      clearTimeout(timer);
      if (!native && this.pending === pending) {
        this.pending = undefined;
        if (!this.state.error) this.publish({ busy: false, error: "" });
      }
    }
  };
}
