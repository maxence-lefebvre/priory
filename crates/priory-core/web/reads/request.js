// What a new task starts from: the workflows, the bands and the defaults. The factory may reload.
export const requestKey = () => ["request"];
export const request = { name: "request", read: (api, _, signal) => api("/api/request", signal) };
