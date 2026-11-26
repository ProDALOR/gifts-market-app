import { ApiFailure } from "./api-error.ts";
import type { WebApp, OptionalSchema } from "./telegram.ts";
import type { MarketSelection } from "./market-selection.ts";
import type { PortfolioReadQuery } from "./portfolio/pager.ts";
import type { SessionRequest } from "./telegram-session.ts";
export type ShareTarget = OptionalSchema<
  "ShareTarget",
  | { type: "gift"; slug: string; currency_id: string }
  | { type: "market"; selection: MarketSelection }
  | { type: "portfolio"; query: PortfolioReadQuery }
>;
export type SharedMessage = OptionalSchema<
  "SharedMessage",
  {
    id?: string;
    expires_at: number;
    inline_query: string;
  }
>;
export type SharingState = { busy: boolean; error: string };
/** No client report cache: the authenticated backend coalesces and versions prepared reports. */
export class SharingController {
  private state: SharingState = { busy: false, error: "" };
  private listeners = new Set<() => void>();
  private pending?: AbortController;
  constructor(
    private app: WebApp | undefined,
    private request: SessionRequest,
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
    if (!this.app?.switchInlineQuery) return false;
    try {
      this.app.switchInlineQuery(query, ["users", "groups", "channels"]);
      return true;
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
      if (
        !Number.isFinite(result.expires_at) ||
        result.expires_at * 1000 <= Date.now() + 1000
      )
        throw new Error("This share expired. Please try again.");
      if (
        result.id &&
        this.app?.shareMessage &&
        this.app.isVersionAtLeast?.("8.0")
      ) {
        // A false callback includes user cancellation. Never open a second chooser after it.
        try {
          this.app.shareMessage(result.id, () => {
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
      if (!this.inline(result.inline_query))
        throw new Error("Open or update Telegram to share messages.");
    } catch (error) {
      // Preserve gift sharing with Mini App-only/older servers and public launches.
      // Portfolio/market fallbacks need the exact cached token to retain their selection.
      if (
        current() &&
        target.type === "gift" &&
        this.app?.switchInlineQuery &&
        (error instanceof TypeError ||
          (error instanceof ApiFailure &&
            ["unavailable", "unauthorized", "not_found", "timed_out"].includes(
              error.code,
            ))) &&
        this.inline(`${target.slug} ${target.currency_id}`)
      ) {
        // The classic gift query is the fallback for servers without prepared sharing.
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
