// A value React reads with useSyncExternalStore and plain code sets: set notifies only on a change.
export function store(initial, same = Object.is) {
  let value = initial;
  const listeners = new Set();
  return {
    get: () => value,
    set(next) {
      if (same(value, next)) return;
      value = next;
      listeners.forEach((listener) => listener());
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}
