// The events that reached no task, by resource, and those no agent read.
export const inbox = () => ({ queryKey: ["inbox"], meta: { read: (api, signal) => api("/api/inbox?unrouted=1", signal) } });
