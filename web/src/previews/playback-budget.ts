type Request = {
  priority: number;
  granted: boolean;
  change: (granted: boolean, preempted: boolean) => void;
};
/** Limit actual players independently of cached downloads. Higher priorities preempt autoplay. */
export class PlaybackBudget {
  private requests = new Set<Request>();
  private suspensions = 0;
  constructor(private limit = 6) {}
  request(priority: number, change: Request["change"]) {
    const entry = { priority, change, granted: false };
    this.requests.add(entry);
    this.reconcile();
    return () => {
      if (!this.requests.delete(entry)) return;
      if (entry.granted) change(false, false);
      this.reconcile();
    };
  }
  suspendBackground() {
    this.suspensions++;
    this.reconcile();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.suspensions--;
      this.reconcile();
    };
  }
  private reconcile() {
    const eligible = [...this.requests].filter(
      (item) => !this.suspensions || item.priority >= 100,
    );
    const winners = new Set(
      eligible.sort((a, b) => b.priority - a.priority).slice(0, this.limit),
    );
    // Stop losers before admitting replacements, including synchronous callbacks.
    for (const item of this.requests)
      if (item.granted && !winners.has(item)) {
        item.granted = false;
        item.change(false, true);
      }
    for (const item of winners)
      if (!item.granted) {
        item.granted = true;
        item.change(true, false);
      }
  }
}
export const playbackBudget = new PlaybackBudget();
