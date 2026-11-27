import type { WebApp } from "./telegram.ts";
import { targetInlineQuery, targetWebsiteUrl } from "./targets.ts";
import type { Target } from "./targets.ts";
export type SharingState = { busy: boolean; error: string };
/** Sharing never creates stored links, sessions or expiring prepared IDs. */
export class SharingController {
  private state: SharingState = { busy: false, error: "" };
  private listeners = new Set<() => void>();
  constructor(
    private app: WebApp | undefined,
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
  cancel = () => this.publish({ busy: false, error: "" });
  share = async (target: Target) => {
    if (this.state.busy) return;
    this.publish({ busy: true, error: "" });
    try {
      const query = targetInlineQuery(target);
      let opened = false;
      if ([...query].length <= 256 && this.app?.switchInlineQuery) {
        try {
          this.app.switchInlineQuery(query, [
            "users",
            "bots",
            "groups",
            "channels",
          ]);
          opened = true;
        } catch {}
      }
      if (!opened) {
        if (!this.app?.openTelegramLink || !this.baseUrl)
          throw Error("Open or update Telegram to share this selection.");
        const url = new URL("https://t.me/share/url");
        const website = targetWebsiteUrl(target, this.baseUrl);
        if ([...website].length > 4096)
          throw Error(
            "Select fewer attributes to share this selection in Telegram.",
          );
        url.searchParams.set("url", website);
        this.app.openTelegramLink(url.href);
      }
      this.publish({ busy: false, error: "" });
    } catch (error) {
      this.publish({
        busy: false,
        error:
          error instanceof Error
            ? error.message
            : "Could not share this selection.",
      });
    }
  };
}
