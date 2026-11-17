export class QueueFullError extends Error {
  constructor() {
    super("Media queue is full");
  }
}
type Job = { priority: number; start: () => void };
/** Bound producers, preserve priority/FIFO order and remove abandoned queued work. */
export class TaskQueue {
  private active = 0;
  private waiting: Job[] = [];
  constructor(
    private concurrency = 4,
    private capacity = 128,
  ) {}
  run<T>(
    work: () => Promise<T>,
    signal: AbortSignal,
    priority = 0,
  ): Promise<T> {
    signal.throwIfAborted();
    if (this.active >= this.concurrency && this.waiting.length >= this.capacity)
      return Promise.reject(new QueueFullError());
    return new Promise((resolve, reject) => {
      let started = false;
      const cancel = () => {
        if (started) return;
        this.waiting = this.waiting.filter((item) => item !== job);
        signal.removeEventListener("abort", cancel);
        reject(signal.reason);
      };
      const job: Job = {
        priority,
        start: () => {
          started = true;
          this.active++;
          void (async () => {
            try {
              signal.throwIfAborted();
              resolve(await work());
            } catch (error) {
              reject(error);
            } finally {
              signal.removeEventListener("abort", cancel);
              this.active--;
              this.pump();
            }
          })();
        },
      };
      signal.addEventListener("abort", cancel, { once: true });
      this.waiting.push(job);
      this.pump();
    });
  }
  private pump() {
    this.waiting.sort((a, b) => b.priority - a.priority);
    while (this.active < this.concurrency && this.waiting.length)
      this.waiting.shift()!.start();
  }
}
