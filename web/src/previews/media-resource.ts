import { ResourceCache } from "../resources/cache.ts";
import { QueueFullError } from "../resources/task-queue.ts";

export class MediaHttpError extends Error {
  readonly retryable: boolean;
  constructor(
    readonly status: number,
    readonly retryAfterMs?: number,
  ) {
    super("Media is unavailable");
    this.retryable = status === 429 || status >= 500;
  }
}
export class MediaNetworkError extends Error {}
/** One deadline shared by every consumer, including across remounts. */
export class MediaFailure extends Error {
  constructor(
    readonly retryAt: number,
    readonly retryable: boolean,
  ) {
    super("Media is unavailable");
  }
}
function cacheFailure(error: unknown) {
  if (
    error instanceof QueueFullError ||
    (error instanceof DOMException && error.name === "AbortError")
  )
    return;
  const transient =
    error instanceof MediaNetworkError ||
    (error instanceof MediaHttpError && error.retryable);
  const failure =
    error instanceof MediaFailure
      ? error
      : new MediaFailure(
          Date.now() +
            (error instanceof MediaHttpError && error.retryAfterMs !== undefined
              ? error.retryAfterMs
              : transient
                ? 2000
                : 60_000),
          transient,
        );
  return { error: failure, expires: failure.retryAt };
}
/** Successful values and failures share the same bounded retention and cancellation. */
export class MediaResource<T> extends ResourceCache<T> {
  constructor(
    load: (
      key: string,
      signal: AbortSignal,
      priority: number,
    ) => Promise<{ value: T; bytes: number }>,
    budget?: number,
    concurrency?: number,
    maxAge?: number,
  ) {
    super(load, budget, concurrency, maxAge, undefined, cacheFailure);
  }
}

export function wait(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const aborted = () => {
      clearTimeout(timer);
      reject(signal.reason);
    };
    const timer = setTimeout(
      () => {
        signal.removeEventListener("abort", aborted);
        resolve();
      },
      Math.min(ms, 2_147_483_647),
    );
    if (signal.aborted) aborted();
    else signal.addEventListener("abort", aborted, { once: true });
  });
}
/** Visible image recovery may wait; it never occupies a producer/transfer slot while cooling down. */
export async function retryMedia<T>(
  read: () => Promise<T>,
  signal: AbortSignal,
): Promise<T> {
  for (;;) {
    signal.throwIfAborted();
    try {
      return await read();
    } catch (error) {
      signal.throwIfAborted();
      if (!(error instanceof MediaFailure) || !error.retryable) throw error;
      do {
        await wait(Math.max(0, error.retryAt - Date.now()), signal);
      } while (Date.now() < error.retryAt);
    }
  }
}
