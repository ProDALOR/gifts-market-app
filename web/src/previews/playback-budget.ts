export type PlaybackGroup =
  "collection" | "selection" | "interaction" | "detail";
type Request = {
  priority: number;
  group: PlaybackGroup;
  turn: number;
  granted: boolean;
  playingSince?: number;
  change: (granted: boolean, preempted: boolean) => void;
};
/** ready() starts one turn when the first frame is visible; late/canceled calls are ignored. */
export type PlaybackLease = { cancel: () => void; ready: () => void };
const TURN_MS = 4000;

/** Conservative device tiers, independent of viewport resizing and individual animations. */
export function playbackCapacity({
  cores = 4,
  memory,
}: { cores?: number; memory?: number } = {}) {
  if (cores <= 2 || (memory !== undefined && memory <= 2)) return 6;
  if (cores <= 4 || (memory !== undefined && memory <= 4)) return 10;
  return 18;
}

/** Share capacity across visible sections; rotate equal-priority waiters within each section. */
export class PlaybackBudget {
  private requests = new Set<Request>();
  private suspensions = new Set<{ all: boolean }>();
  private nextTurn = 0;
  private timer?: ReturnType<typeof setTimeout>;
  private deadline?: number;
  private reconciling = false;
  private dirty = false;
  constructor(
    private limit = playbackCapacity(),
    private now = () => performance.now(),
  ) {
    this.validateLimit(limit);
  }
  request(
    { priority, group }: Pick<Request, "priority" | "group">,
    change: Request["change"],
  ): PlaybackLease {
    const entry: Request = {
      priority,
      group,
      change,
      turn: this.nextTurn++,
      granted: false,
    };
    this.requests.add(entry);
    this.reconcile();
    return {
      cancel: () => {
        if (!this.requests.delete(entry)) return;
        if (entry.granted) {
          entry.granted = false;
          change(false, false);
        }
        this.reconcile();
      },
      ready: () => {
        if (
          !this.requests.has(entry) ||
          !entry.granted ||
          entry.playingSince !== undefined
        )
          return;
        entry.playingSince = this.now();
        this.reconcile();
      },
    };
  }
  private validateLimit(limit: number) {
    if (!Number.isInteger(limit) || limit < 1)
      throw new Error("Playback capacity must be a positive integer");
  }
  setLimit(limit: number) {
    this.validateLimit(limit);
    if (limit === this.limit) return;
    this.limit = limit;
    this.reconcile();
  }
  suspendBackground() {
    return this.suspend(false);
  }
  suspendAll() {
    return this.suspend(true);
  }
  private suspend(all: boolean) {
    const suspension = { all };
    this.suspensions.add(suspension);
    this.reconcile();
    return () => {
      if (!this.suspensions.delete(suspension)) return;
      this.reconcile();
    };
  }
  private reconcile() {
    if (this.reconciling) {
      this.dirty = true;
      return;
    }
    this.reconciling = true;
    try {
      do {
        this.dirty = false;
        const paused = [...this.suspensions].some((item) => item.all);
        const eligible = [...this.requests]
          .filter(
            (item) =>
              !paused && (!this.suspensions.size || item.group === "detail"),
          )
          .sort((a, b) => b.priority - a.priority || a.turn - b.turn);
        const groups = new Map<PlaybackGroup, Request[]>();
        for (const item of eligible) {
          const queue = groups.get(item.group);
          if (queue) queue.push(item);
          else groups.set(item.group, [item]);
        }
        const winners = new Set<Request>();
        for (
          let round = 0;
          winners.size < Math.min(this.limit, eligible.length);
          round++
        ) {
          for (const queue of groups.values()) {
            if (winners.size === this.limit) break;
            if (queue[round]) winners.add(queue[round]);
          }
        }
        // Release all old players before creating replacements, even when callbacks cancel work.
        for (const item of this.requests)
          if (item.granted && !winners.has(item)) {
            item.granted = false;
            item.playingSince = undefined;
            item.change(false, true);
          }
        if (this.dirty) continue;
        for (const item of winners)
          if (!item.granted) {
            item.granted = true;
            item.change(true, false);
            if (this.dirty) break;
          }
        if (this.dirty) continue;
        const waitingTiers = new Set(
          eligible
            .filter((item) => !winners.has(item))
            .map((item) => `${item.group}:${item.priority}`),
        );
        const deadlines = [...winners].flatMap((item) =>
          item.playingSince !== undefined &&
          waitingTiers.has(`${item.group}:${item.priority}`)
            ? [item.playingSince + TURN_MS]
            : [],
        );
        this.schedule(deadlines.length ? Math.min(...deadlines) : undefined);
      } while (this.dirty);
    } finally {
      this.reconciling = false;
    }
  }
  private schedule(deadline?: number) {
    if (this.deadline === deadline) return;
    clearTimeout(this.timer);
    this.timer = undefined;
    this.deadline = deadline;
    if (deadline === undefined) return;
    this.timer = setTimeout(
      () => {
        this.timer = undefined;
        this.deadline = undefined;
        const now = this.now();
        for (const item of this.requests)
          if (
            item.granted &&
            item.playingSince !== undefined &&
            item.playingSince + TURN_MS <= now
          ) {
            item.turn = this.nextTurn++;
            // With fewer waiters than players, unchanged winners keep playing for another turn.
            item.playingSince = now;
          }
        this.reconcile();
      },
      Math.max(0, deadline - this.now()),
    );
  }
}
const device =
  typeof navigator === "undefined"
    ? undefined
    : (navigator as Navigator & { deviceMemory?: number });
export const playbackBudget = new PlaybackBudget(
  playbackCapacity({
    cores: device?.hardwareConcurrency,
    memory: device?.deviceMemory,
  }),
);
