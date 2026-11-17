import { QueueFullError } from "./task-queue.ts";
/** Owns one renderer across brief hover transitions. Visibility loss is immediate. */
export interface PreviewPlayer {
  play(): void;
  pause(): void;
  destroy(): void;
  ready(): boolean;
  listen(
    event: "DOMLoaded" | "data_failed" | "error",
    callback: () => void,
  ): void;
}
export class AnimationLifecycle {
  private player?: PreviewPlayer;
  private load?: AbortController;
  private timer?: ReturnType<typeof setTimeout>;
  private active = false;
  constructor(
    private create: (signal: AbortSignal) => Promise<PreviewPlayer>,
    private show: (ready: boolean) => void,
    private grace = 350,
    private failed: () => void = () => {},
  ) {}
  setActive(active: boolean, retain = true) {
    this.active = active;
    clearTimeout(this.timer);
    if (!active) {
      this.show(false);
      try {
        this.player?.pause();
      } catch {
        this.stop();
        return;
      }
      // Don't keep a pending network load alive for a pointer that already left.
      if (!retain || !this.player) this.stop();
      else this.timer = setTimeout(() => this.stop(), this.grace);
      return;
    }
    if (this.player) {
      this.resume();
      return;
    }
    if (this.load) return;
    const load = new AbortController();
    this.load = load;
    void this.create(load.signal)
      .then((player) => {
        if (load.signal.aborted || this.load !== load) {
          player.destroy();
          return;
        }
        this.player = player;
        player.listen("DOMLoaded", () => {
          if (this.player === player) this.resume();
        });
        player.listen("data_failed", () => {
          if (this.player === player) this.fail();
        });
        player.listen("error", () => {
          if (this.player === player) this.fail();
        });
        this.resume();
      })
      .catch((error) => {
        if (this.load !== load) return;
        const retry =
          error instanceof QueueFullError &&
          this.active &&
          !load.signal.aborted;
        this.stop();
        if (retry) this.timer = setTimeout(() => this.setActive(true), 150);
        else this.failed();
      });
  }
  private resume() {
    if (this.active && this.player?.ready()) {
      try {
        const player = this.player;
        player.play();
        if (this.player === player && this.active) this.show(true);
      } catch {
        this.fail();
      }
    }
  }
  private fail() {
    this.stop();
    this.failed();
  }
  stop() {
    clearTimeout(this.timer);
    this.load?.abort();
    this.load = undefined;
    const player = this.player;
    this.player = undefined;
    this.show(false);
    // Lottie can emit events from destroy; release ownership before calling it.
    try {
      player?.destroy();
    } catch {
      /* Fallback already restored. */
    }
  }
}
