/** Per-device preferences remain usable when persistent storage is disabled. */
export function createPreference<T>(
  key: string,
  normalize: (value: string | null) => T,
) {
  const read = () => {
    try {
      return localStorage.getItem(key);
    } catch {
      return undefined;
    }
  };
  let value = normalize(read() ?? null);
  let localOnly = false;
  const listeners = new Set<() => void>();
  const publish = (next: T) => {
    if (Object.is(value, next)) return;
    value = next;
    listeners.forEach((listener) => listener());
  };
  const storage = (event: StorageEvent) => {
    try {
      if (event.storageArea && event.storageArea !== localStorage) return;
    } catch {
      return;
    }
    if (event.key === key || event.key === null) {
      localOnly = false;
      publish(normalize(event.newValue));
    }
  };
  return {
    getSnapshot: () => value,
    subscribe(listener: () => void) {
      listeners.add(listener);
      if (listeners.size === 1) {
        window.addEventListener("storage", storage);
        const saved = read();
        if (!localOnly && saved !== undefined) publish(normalize(saved));
      }
      return () => {
        listeners.delete(listener);
        if (!listeners.size) window.removeEventListener("storage", storage);
      };
    },
    set(next: T, serialized: string) {
      try {
        localStorage.setItem(key, serialized);
        localOnly = false;
      } catch {
        localOnly = true;
      }
      publish(next);
    },
  };
}
