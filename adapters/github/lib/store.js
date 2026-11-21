// The extension's own IndexedDB: its origin is chrome-extension://<id>, which GitHub's pages and
// content scripts cannot open. It keeps the address and token, the last read per pull request and
// the quiz scores.

const open = () =>
  new Promise((resolve, reject) => {
    const request = indexedDB.open("priory", 1);
    request.onupgradeneeded = () => request.result.createObjectStore("kv");
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });

async function run(mode, use) {
  const db = await open();
  return new Promise((resolve, reject) => {
    const transaction = db.transaction("kv", mode);
    const request = use(transaction.objectStore("kv"));
    transaction.oncomplete = () => resolve(request.result);
    transaction.onerror = () => reject(transaction.error);
  }).finally(() => db.close());
}

export const get = (key) => run("readonly", (store) => store.get(key));
export const put = (key, value) => run("readwrite", (store) => store.put(value, key));
export const remove = (key) => run("readwrite", (store) => store.delete(key));
