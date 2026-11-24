import { ApiFailure } from "../market.ts";
import { retryDelay } from "../retry.ts";
export type Query<T> = (
  signal: AbortSignal,
  revision: (value: string) => void,
) => Promise<T>;
/** One request at a time. Invalidations coalesce behind it instead of cancelling useful work. */
export class CatalogResource<T> {
  value?: T;
  revision?: string;
  error = "";
  loading = false;
  active = false;
  target?: string | null;
  hasTarget = false;
  private checked?: string | null;
  private checkedTarget = false;
  private forced = false;
  private blocked = false;
  private failures = 0;
  private retry?: ReturnType<typeof setTimeout>;
  private pending?: AbortController;
  private disposed = false;
  constructor(
    private query: Query<T>,
    private changed: () => void,
    private admit: () => void | boolean,
    private random: () => number,
  ) {}
  get fresh() {
    return (
      this.value !== undefined &&
      !this.forced &&
      (!this.hasTarget ||
        this.revision === this.target ||
        (this.checkedTarget && this.checked === this.target))
    );
  }
  setTarget(value: string | null) {
    if (this.hasTarget && this.target === value) return;
    this.target = value;
    this.hasTarget = true;
    this.blocked = false;
    this.failures = 0;
    this.error = "";
    clearTimeout(this.retry);
    this.retry = undefined;
    this.ensure();
  }
  activate(active: boolean) {
    const previous = this.active;
    this.active = active;
    if (!active) {
      clearTimeout(this.retry);
      this.retry = undefined;
    } else {
      if (!previous) this.blocked = false;
      this.ensure();
    }
  }
  reload() {
    this.blocked = false;
    this.forced = true;
    this.failures = 0;
    clearTimeout(this.retry);
    this.retry = undefined;
    this.ensure();
  }
  dispose() {
    this.disposed = true;
    this.pending?.abort();
    clearTimeout(this.retry);
  }
  private ensure() {
    if (
      this.disposed ||
      !this.active ||
      this.loading ||
      this.retry ||
      this.blocked ||
      this.fresh
    )
      return;
    if (this.hasTarget && this.target === null) {
      this.error = "Collection is no longer available";
      this.blocked = true;
      this.changed();
      return;
    }
    if (this.admit() === false) return;
    const stop = new AbortController();
    this.pending = stop;
    this.loading = true;
    this.error = "";
    this.forced = false;
    const target = this.target,
      hadTarget = this.hasTarget;
    let revision: string | undefined;
    this.changed();
    void this.query(stop.signal, (value) => {
      revision = value;
    })
      .then((value) => {
        if (stop.signal.aborted) return;
        const moved = target !== this.target || hadTarget !== this.hasTarget;
        if (moved && revision !== this.target) return;
        this.value = value;
        this.revision = revision;
        this.checked = this.target;
        this.checkedTarget = this.hasTarget;
        this.error = "";
        this.failures = 0;
      })
      .catch((error) => {
        if (
          stop.signal.aborted ||
          target !== this.target ||
          hadTarget !== this.hasTarget
        )
          return;
        this.error = error instanceof Error ? error.message : String(error);
        const transient =
          !(error instanceof ApiFailure) ||
          ["unavailable", "capacity_exceeded", "restart_read"].includes(
            error.code,
          );
        this.blocked = true;
        if (transient && this.active)
          this.retry = setTimeout(
            () => {
              this.retry = undefined;
              this.blocked = false;
              this.ensure();
            },
            retryDelay(this.failures++, this.random),
          );
      })
      .finally(() => {
        if (stop.signal.aborted) return;
        this.pending = undefined;
        this.loading = false;
        this.changed();
        this.ensure();
      });
  }
}
