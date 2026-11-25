// A batch, by its id or by its planning task's key. `version` is a go ask's revision, so a plan read
// for one revision is never drawn beside another.
export const batchKey = (which, version = null) => ["batch", which, version];
// A `batch:YYYY-MM-DD-n` key is path-safe, and the server does not decode the path.
export const batch = { name: "batch", read: (api, [, which], signal) => api(`/api/batches/${which}`, signal) };
