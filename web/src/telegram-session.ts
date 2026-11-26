import { ApiFailure } from "./api-error.ts";
import {
  cachedSession,
  launchFingerprint,
  pauseSession,
  saveSession,
  sessionPaused,
} from "./telegram.ts";
import type {
  Identity,
  Session,
  TelegramConfig,
  TelegramContext,
} from "./telegram.ts";

export type SignInStatus =
  | "idle"
  | "connecting"
  | "retrying"
  | "connected"
  | "signed_out"
  | "unavailable"
  | "reopen";
export type TelegramSessionState = {
  status: SignInStatus;
  config?: TelegramConfig;
  identity?: Identity;
  error?: string;
};
export type SessionRequest = <T>(
  path: string,
  body?: unknown,
  signal?: AbortSignal,
  token?: string,
) => Promise<T>;
const RETRY_DELAYS = [1000, 2000, 4000, 8000, 15000, 30000];

/** One bounded sign-in lifecycle per launch. Aborted work cannot publish or save tokens. */
export class TelegramSessionController {
  private snapshot: TelegramSessionState = { status: "idle" };
  private listeners = new Set<() => void>();
  private running = false;
  private token?: string;
  private sessionRequests = new AbortController();
  private pending?: AbortController;
  private timer?: ReturnType<typeof setTimeout>;
  private failures = 0;
  private credential?: { botId: string; launch: string };
  constructor(
    private context: TelegramContext,
    private request: SessionRequest,
    private active = () => true,
  ) {}
  current = () => this.snapshot;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  start() {
    if (this.running) return;
    this.running = true;
    if (["reopen", "signed_out"].includes(this.snapshot.status)) return;
    if (
      this.snapshot.identity &&
      Date.parse(this.snapshot.identity.expires_at) <= Date.now()
    ) {
      this.expired();
      return;
    }
    void this.read();
  }
  stop() {
    this.running = false;
    this.sessionRequests.abort();
    this.sessionRequests = new AbortController();
    this.cancel();
  }
  resume = () => {
    if (!this.running) return;
    const identity = this.snapshot.identity;
    if (identity && Date.parse(identity.expires_at) <= Date.now())
      this.expired();
    else if (
      this.running &&
      !this.pending &&
      !this.timer &&
      ["retrying", "unavailable"].includes(this.snapshot.status)
    ) {
      this.failures = 0;
      void this.read();
    }
  };
  retry = () => {
    if (!this.running || this.snapshot.status === "reopen") return;
    this.cancel();
    if (this.credential) pauseSession(this.credential.botId, null);
    this.failures = 0;
    void this.read();
  };
  logout = () => {
    this.token = undefined;
    this.sessionRequests.abort();
    this.sessionRequests = new AbortController();
    this.cancel();
    if (this.credential) {
      const { botId, launch } = this.credential;
      saveSession(botId, launch, null);
      pauseSession(botId, launch);
    }
    this.publish({
      status: "signed_out",
      identity: undefined,
      error: undefined,
    });
  };
  /** Credentials stay inside the session controller, including when browser storage is blocked. */
  authorized: SessionRequest = async <T>(
    path: string,
    body?: unknown,
    signal?: AbortSignal,
  ): Promise<T> => {
    if (
      !this.running ||
      this.snapshot.status !== "connected" ||
      !this.token ||
      !this.snapshot.identity ||
      Date.parse(this.snapshot.identity.expires_at) <= Date.now()
    ) {
      if (this.snapshot.identity) this.expired();
      throw new ApiFailure(
        "unauthorized",
        "Sign in to Telegram before sharing.",
      );
    }
    const token = this.token;
    const session = this.sessionRequests.signal;
    const pending = new AbortController();
    const abort = () => pending.abort();
    session.addEventListener("abort", abort, { once: true });
    signal?.addEventListener("abort", abort, { once: true });
    if (session.aborted || signal?.aborted) abort();
    try {
      const result = await this.request<T>(path, body, pending.signal, token);
      if (pending.signal.aborted)
        throw new DOMException("Sharing cancelled", "AbortError");
      return result;
    } catch (error) {
      if (
        !pending.signal.aborted &&
        this.token === token &&
        error instanceof ApiFailure &&
        error.code === "unauthorized"
      )
        this.expired();
      throw error;
    } finally {
      session.removeEventListener("abort", abort);
      signal?.removeEventListener("abort", abort);
    }
  };
  private cancel() {
    clearTimeout(this.timer);
    this.timer = undefined;
    const previous = this.pending;
    this.pending = undefined;
    previous?.abort();
  }
  private publish(change: Partial<TelegramSessionState>) {
    this.snapshot = { ...this.snapshot, ...change };
    for (const listener of this.listeners) listener();
  }
  private expired() {
    this.token = undefined;
    this.sessionRequests.abort();
    this.sessionRequests = new AbortController();
    this.cancel();
    if (this.credential)
      saveSession(this.credential.botId, this.credential.launch, null);
    this.publish({
      status: "reopen",
      identity: undefined,
      error: "Close and reopen this Mini App in Telegram to sign in again.",
    });
  }
  private later(message: string) {
    const delay = RETRY_DELAYS[this.failures++];
    if (delay !== undefined)
      this.timer = setTimeout(() => {
        this.timer = undefined;
        void this.read();
      }, delay);
    this.publish({
      status: delay === undefined ? "unavailable" : "retrying",
      error: message,
    });
  }
  private async read() {
    if (!this.running || this.pending) return;
    if (!this.active()) {
      this.publish({ status: "retrying" });
      return;
    }
    const attempt = new AbortController();
    this.pending = attempt;
    const current = () =>
      this.running && this.pending === attempt && !attempt.signal.aborted;
    const timeout = setTimeout(() => {
      if (this.pending !== attempt || !this.running) return;
      this.pending = undefined;
      attempt.abort();
      this.later("Telegram sign-in is temporarily unavailable.");
    }, 10000);
    attempt.signal.addEventListener("abort", () => clearTimeout(timeout), {
      once: true,
    });
    this.publish({ status: "connecting", error: undefined });
    try {
      const config = await this.request<TelegramConfig>(
        "/telegram",
        undefined,
        attempt.signal,
      );
      if (!current()) return;
      this.publish({ config });
      if (!this.context.initData) {
        this.publish({ status: "idle" });
        return;
      }
      if (!config.auth_enabled || !config.bot_id) {
        this.later("Telegram sign-in is not ready yet.");
        return;
      }
      const launch = await launchFingerprint(this.context);
      if (!current()) return;
      this.credential = { botId: config.bot_id, launch };
      if (sessionPaused(config.bot_id, launch)) {
        this.publish({ status: "signed_out", identity: undefined });
        return;
      }
      const saved = cachedSession(config.bot_id, launch);
      let identity: Identity | undefined;
      let token = saved?.token;
      if (saved) {
        try {
          identity = await this.request<Identity>(
            "/auth/me",
            undefined,
            attempt.signal,
            saved.token,
          );
        } catch (error) {
          if (!current()) return;
          if (!(error instanceof ApiFailure) || error.code !== "unauthorized")
            throw error;
          saveSession(config.bot_id, launch, null);
        }
      }
      if (!current()) return;
      if (!identity) {
        const session = await this.request<Session>(
          "/auth/telegram",
          { init_data: this.context.initData },
          attempt.signal,
        );
        if (!current()) return;
        token = session.token;
        saveSession(config.bot_id, launch, session.token);
        identity = { user: session.user, expires_at: session.expires_at };
      }
      if (!current()) return;
      const remaining = Date.parse(identity.expires_at) - Date.now();
      if (!Number.isFinite(remaining) || remaining <= 0) {
        this.expired();
        return;
      }
      this.token = token;
      this.failures = 0;
      this.timer = setTimeout(() => this.expired(), remaining);
      this.publish({ status: "connected", identity, error: undefined });
    } catch (error) {
      if (this.pending !== attempt || !this.running) return;
      if (error instanceof ApiFailure && error.code === "unauthorized")
        this.expired();
      else this.later("Telegram sign-in is temporarily unavailable.");
    } finally {
      clearTimeout(timeout);
      if (this.pending === attempt) this.pending = undefined;
    }
  }
}
