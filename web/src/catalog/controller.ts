import { catalog, collectionCatalog } from "../market.ts";
import type {
  Catalog,
  CatalogOption,
  CatalogVersions,
  Collection,
} from "../market.ts";
import { liveSocket } from "../live-socket.ts";
import type { LiveSocket } from "../live-socket.ts";
import { CatalogResource } from "./resource.ts";
const emptyOptions: ReadonlyMap<string, CatalogOption> = new Map();
const emptyCatalog: Catalog = {
  collections: [],
  venues: [],
  currencies: [],
  books: [],
};
export type CatalogState = {
  selectedId: string;
  types: Collection[];
  venues: Catalog["venues"];
  directory: Catalog["books"];
  item?: Collection;
  options: ReadonlyMap<string, CatalogOption>;
  previewOptions: ReadonlyMap<string, CatalogOption>;
  names: Map<string, string>;
  catalogLoading: boolean;
  startupError: string;
  optionsError: string;
  optionsLoading: boolean;
};
type Options = ReadonlyMap<string, CatalogOption>;
type Entry = { resource: CatalogResource<Options>; used: number };
export type CatalogDependencies = {
  catalog: typeof catalog;
  collection: typeof collectionCatalog;
  socket: Pick<LiveSocket, "subscribe" | "update" | "retry">;
  random: () => number;
};
/** Owns catalog requests, revision reconciliation and bounded retention; independent of React. */
export class CatalogController {
  private directory: CatalogResource<Catalog>;
  private collections = new Map<string, Entry>();
  private listeners = new Set<() => void>();
  private selectedId = "";
  private previewIds: string[] = [];
  private previewMaps: Options[] = [];
  private watches = new Map<
    string,
    { signature: string; stop: () => void; error: string }
  >();
  private sequence = 0;
  private manifest?: CatalogVersions;
  private fallback = false;
  private fallbackTimer?: ReturnType<typeof setTimeout>;

  private started = false;
  private closed = false;
  private liveError = "";

  private materialized = emptyCatalog;
  private types: Collection[] = [];
  private state: CatalogState = {
    selectedId: "",
    types: [],
    venues: [],
    directory: [],
    options: emptyOptions,
    previewOptions: emptyOptions,
    names: new Map(),
    catalogLoading: true,
    startupError: "",
    optionsError: "",
    optionsLoading: false,
  };
  private deps: CatalogDependencies;
  constructor(deps: Partial<CatalogDependencies> = {}) {
    this.deps = {
      catalog,
      collection: collectionCatalog,
      socket: liveSocket,
      random: Math.random,
      ...deps,
    };
    this.directory = new CatalogResource(
      this.deps.catalog,
      () => this.reconcile(),
      () => {},
      this.deps.random,
    );
  }
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  start() {
    if (this.started || this.closed) return;
    this.started = true;
    this.updateWatch();
    // HTTP remains usable when a proxy or older deployment cannot establish a catalog watch.
    this.fallbackTimer = setTimeout(() => {
      this.fallback = true;
      this.reconcile();
    }, 1000);
    this.reconcile();
  }
  select(id: string) {
    if (this.selectedId === id || this.closed) return;
    this.selectedId = id;
    const entry = this.collections.get(id);
    if (entry) entry.used = ++this.sequence;
    this.updateWatch();
    this.reconcile();
  }
  previews(ids: readonly string[]) {
    const next = [...new Set(ids.filter(Boolean))].slice(0, 32);
    if (JSON.stringify(next) === JSON.stringify(this.previewIds) || this.closed)
      return;
    this.previewIds = next;
    for (const id of next) {
      const entry = this.collections.get(id);
      if (entry) entry.used = ++this.sequence;
    }
    this.updateWatch();
    this.reconcile();
  }
  retryCatalog = () => {
    if (this.liveError)
      for (const id of this.watches.keys()) this.deps.socket.retry(id);
    if (this.directory.error || this.directory.value === undefined)
      this.directory.reload();
  };
  retryOptions = () => {
    if (this.liveError)
      for (const id of this.watches.keys()) this.deps.socket.retry(id);
    this.collections.get(this.selectedId)?.resource.reload();
    if (this.directory.error) this.directory.reload();
  };
  close() {
    this.closed = true;
    clearTimeout(this.fallbackTimer);
    for (const watch of this.watches.values()) watch.stop();
    this.watches.clear();
    this.directory.dispose();
    for (const entry of this.collections.values()) entry.resource.dispose();
    this.listeners.clear();
  }
  private wanted() {
    return [
      ...new Set([this.selectedId, ...this.previewIds].filter(Boolean)),
    ].slice(0, 32);
  }
  private capacity() {
    return Math.max(8, this.wanted().length);
  }
  private watchError() {
    return [...this.watches.values()].find((watch) => watch.error)?.error || "";
  }
  private updateWatch() {
    if (!this.started || this.closed) return;
    const retained = [...this.collections]
      .filter(([, entry]) => entry.resource.value !== undefined)
      .sort((a, b) => b[1].used - a[1].used)
      .map(([id]) => id);
    const ids = [...new Set([...this.wanted(), ...retained])]
      .slice(0, this.capacity())
      .sort();
    const count = Math.max(1, Math.ceil(ids.length / 8));
    for (const [id, watch] of this.watches) {
      const index = id === "catalog" ? 0 : Number(id.slice(8));
      if (index >= count) {
        watch.stop();
        this.watches.delete(id);
      }
    }
    this.liveError = this.watchError();
    for (let i = 0; i < count; i++) {
      const id = i ? `catalog.${i}` : "catalog";
      const collection_ids = ids.slice(i * 8, i * 8 + 8);
      const signature = JSON.stringify(collection_ids);
      const command = { type: "catalog.watch" as const, id, collection_ids };
      const watch = this.watches.get(id);
      if (watch) {
        if (watch.signature !== signature) {
          watch.signature = signature;
          this.deps.socket.update(command);
        }
        continue;
      }
      const entry = { signature, stop: () => {}, error: "" };
      this.watches.set(id, entry);
      entry.stop = this.deps.socket.subscribe(
        command,
        (frame) => {
          if (this.closed || this.watches.get(id) !== entry) return;
          if (frame.type === "catalog.versions") {
            entry.error = "";
            this.liveError = this.watchError();
            this.fallback = !!this.liveError;
            this.manifest = {
              directory: frame.versions.directory,
              collections: {
                ...this.manifest?.collections,
                ...frame.versions.collections,
              },
            };
            clearTimeout(this.fallbackTimer);
            this.directory.setTarget(frame.versions.directory);
            for (const [key, resource] of this.collections)
              if (Object.hasOwn(frame.versions.collections, key))
                resource.resource.setTarget(frame.versions.collections[key]);
            this.reconcile();
          } else if (frame.type === "error") {
            entry.error = `Catalog live updates: ${frame.error.message}`;
            this.liveError = this.watchError();
            this.fallback = true;
            this.reconcile();
          }
        },
        (status) => {
          if (this.closed || this.watches.get(id) !== entry) return;
          if (status === "Reconnecting") {
            entry.error = "Catalog live updates are reconnecting";
            this.liveError = this.watchError();
          }
          this.publish();
        },
      );
    }
  }
  private entry(id: string) {
    let entry = this.collections.get(id);
    if (entry) return entry;
    const resource = new CatalogResource<Options>(
      async (signal, received) => {
        const page = await this.deps.collection(id, signal, received);
        const rows: CatalogOption[] = [
          ...page.models.map((m) => ({ ...m, kind: "model" as const })),
          ...page.backdrops.map((b) => ({ ...b, kind: "backdrop" as const })),
          ...page.patterns.map((p) => ({ ...p, kind: "pattern" as const })),
        ];
        return new Map(rows.map((row) => [row.id, row]));
      },
      () => this.reconcile(),
      () => this.admit(id),
      this.deps.random,
    );
    entry = { resource, used: ++this.sequence };
    this.collections.set(id, entry);
    return entry;
  }
  private admit(id: string) {
    const others = [...this.collections]
      .filter(([key, e]) => key !== id && e.resource.loading)
      .sort((a, b) => a[1].used - b[1].used);
    if (id !== this.selectedId && others.length >= 2) return false;
    while (others.length >= 2) {
      const [key, entry] = others.shift()!;
      entry.resource.dispose();
      this.collections.delete(key);
    }
  }
  private prune() {
    for (const [id, entry] of this.collections)
      if (
        !this.wanted().includes(id) &&
        !entry.resource.loading &&
        entry.resource.value === undefined
      ) {
        entry.resource.dispose();
        this.collections.delete(id);
      }
    const retained = [...this.collections]
      .filter(([, e]) => e.resource.value !== undefined)
      .sort((a, b) => a[1].used - b[1].used);
    for (const [id, entry] of retained)
      if (!this.wanted().includes(id) && entry.resource.value!.size > 10000) {
        entry.resource.dispose();
        this.collections.delete(id);
      }
    const bounded = retained.filter(
      ([id, e]) => this.collections.has(id) && e.resource.value!.size <= 10000,
    );
    let size = bounded.reduce(
      (total, [, e]) => total + e.resource.value!.size,
      0,
    );
    while (
      bounded.length > this.capacity() ||
      size > (this.capacity() > 8 ? 16000 : 10000)
    ) {
      const index = bounded.findIndex(([id]) => !this.wanted().includes(id));
      if (index < 0) break;
      const [id, entry] = bounded.splice(index, 1)[0];
      size -= entry.resource.value!.size;
      entry.resource.dispose();
      this.collections.delete(id);
    }
  }
  private reconciling = false;
  private reconcile() {
    if (this.closed || this.reconciling) return;
    this.reconciling = true;
    try {
      const directory = this.directory.value;
      if (directory && directory !== this.materialized) {
        const old = new Map(this.types.map((row) => [row.id, row]));
        this.types = directory.collections
          .map((row) => {
            const previous = old.get(row.id);
            return previous && JSON.stringify(previous) === JSON.stringify(row)
              ? previous
              : row;
          })
          .sort((a, b) => a.name.localeCompare(b.name));
        this.materialized = directory;
      }
      const allowed = this.started && (!!this.manifest || this.fallback);
      this.directory.activate(allowed);
      const wanted = this.wanted();
      for (const [id, entry] of this.collections)
        if (
          !wanted.includes(id) ||
          !allowed ||
          !this.types.some((row) => row.id === id)
        )
          entry.resource.activate(false);
      for (const id of wanted) {
        if (!directory || !this.types.some((row) => row.id === id)) continue;
        const entry = this.entry(id);
        const known = this.manifest?.collections;
        if (known && Object.hasOwn(known, id))
          entry.resource.setTarget(known[id]);
        const running = [...this.collections.values()].filter(
          (e) => e.resource.loading,
        ).length;
        // The selected picker may replace abandoned work; previews wait their turn.
        entry.resource.activate(
          allowed &&
            (id === this.selectedId || entry.resource.loading || running < 2) &&
            ((!!known && Object.hasOwn(known, id)) || this.fallback),
        );
      }
      this.prune();
      this.updateWatch();
      this.publish();
    } finally {
      this.reconciling = false;
    }
  }
  private publish() {
    if (this.closed) return;
    const directory = this.directory.value ?? emptyCatalog;
    const item = this.types.find((row) => row.id === this.selectedId);
    const resource = item
      ? this.collections.get(this.selectedId)?.resource
      : undefined;
    const options = resource?.value ?? emptyOptions;
    const previous = this.state;
    const maps = [...this.collections.values()].flatMap((e) =>
      e.resource.value ? [e.resource.value] : [],
    );
    const sameMaps =
      maps.length === this.previewMaps.length &&
      maps.every((map, i) => map === this.previewMaps[i]);
    const previewOptions = sameMaps
      ? previous.previewOptions
      : new Map(maps.flatMap((map) => [...map]));
    this.previewMaps = maps;
    const names =
      previous.types === this.types &&
      previous.venues === directory.venues &&
      previous.previewOptions === previewOptions
        ? previous.names
        : new Map(
            [
              ...this.types,
              ...directory.venues,
              ...previewOptions.values(),
            ].map((row) => [row.id, row.name]),
          );
    const next: CatalogState = {
      selectedId: this.selectedId,
      types: this.types,
      venues: directory.venues,
      directory: directory.books,
      item,
      options,
      previewOptions,
      names,
      catalogLoading:
        this.directory.value === undefined && !this.directory.error,
      startupError: this.directory.error || this.liveError,
      optionsError: resource?.error ?? "",
      optionsLoading:
        !!item &&
        (!!resource?.loading ||
          (resource?.value === undefined && !resource?.error)),
    };
    if (
      Object.keys(next).some(
        (key) =>
          next[key as keyof CatalogState] !==
          previous[key as keyof CatalogState],
      )
    ) {
      this.state = next;
      for (const listener of this.listeners) listener();
    }
  }
}
