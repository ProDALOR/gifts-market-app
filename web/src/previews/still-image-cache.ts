/** Keep a small decoded-image LRU across virtual-card remounts, without retaining React DOM/fibers. */
export class StillImageCache {
  private entries = new Map<
    string,
    { image: HTMLImageElement; bytes: number }
  >();
  private bytes = 0;
  constructor(
    private budget = 24 * 1024 * 1024,
    private limit = 64,
  ) {}
  has(url: string) {
    const entry = this.entries.get(url);
    if (!entry) return false;
    this.entries.delete(url);
    this.entries.set(url, entry);
    return true;
  }
  remember(url: string, source: HTMLImageElement) {
    if (!source.complete || !source.naturalWidth || !source.naturalHeight)
      return;
    if (this.has(url)) return;
    const bytes = source.naturalWidth * source.naturalHeight * 4;
    if (bytes > this.budget) return;
    // cloneNode copies HTML attributes, not React's event handlers or component tree.
    const image = source.cloneNode(false) as HTMLImageElement;
    image.loading = "eager";
    image.removeAttribute("class");
    image.removeAttribute("style");
    this.entries.set(url, { image, bytes });
    this.bytes += bytes;
    while (this.bytes > this.budget || this.entries.size > this.limit) {
      const key = this.entries.keys().next().value!;
      const old = this.entries.get(key)!;
      this.entries.delete(key);
      this.bytes -= old.bytes;
      old.image.removeAttribute("src");
    }
  }
  forget(url: string) {
    const entry = this.entries.get(url);
    if (!entry) return;
    this.entries.delete(url);
    this.bytes -= entry.bytes;
    entry.image.removeAttribute("src");
  }
}
export const stillImages = new StillImageCache();
