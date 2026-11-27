import { ApiFailure } from "./api-error.ts";

export function retryableFailure(error: unknown) {
  return (
    !(error instanceof ApiFailure) ||
    ["unavailable", "capacity_exceeded", "restart_read", "timed_out"].includes(
      error.code,
    )
  );
}
/** Bounded exponential retry with jitter; never used for successful catalog reads. */
export function retryDelay(attempt: number, random = Math.random) {
  return Math.min(
    15000,
    1000 * 2 ** Math.min(attempt, 4) * (0.8 + random() * 0.4),
  );
}
