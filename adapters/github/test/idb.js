// The few IndexedDB calls store.js makes, on one in-memory object store.
const later = (fn) => setImmediate(fn);
export const data = new Map();

globalThis.indexedDB = {
  open() {
    const opening = {};
    const db = {
      close() {},
      transaction() {
        const transaction = {};
        const request = (apply) => {
          const pending = {};
          later(() => {
            pending.result = apply();
            transaction.oncomplete?.();
          });
          return pending;
        };
        transaction.objectStore = () => ({
          get: (key) => request(() => data.get(key)),
          put: (value, key) => request(() => data.set(key, value) && key),
          delete: (key) => request(() => data.delete(key) && undefined),
          clear: () => request(() => data.clear()),
        });
        return transaction;
      },
    };
    later(() => {
      opening.result = db;
      opening.onsuccess();
    });
    return opening;
  },
};
