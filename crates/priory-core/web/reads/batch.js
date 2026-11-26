// A batch, by its id or by its planning task's key. `version` is a go ask's revision, so a plan read
// for one revision is never drawn beside another. The server does not decode the path, and a
// `batch:YYYY-MM-DD-n` key is path-safe.
export const batch = (which, version = null) => ({ queryKey: ["batch", which, version], meta: { read: (api, signal) => api(`/api/batches/${which}`, signal) } });
