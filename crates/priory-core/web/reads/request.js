// What a new task starts from: the workflows, the bands and the defaults. The factory may reload.
export const request = () => ({ queryKey: ["request"], meta: { read: (api, signal) => api("/api/request", signal) } });
