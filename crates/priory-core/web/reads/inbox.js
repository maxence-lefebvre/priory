// The events that reached no task, by resource, and those no agent read.
export const inboxKey = () => ["inbox"];
export const inbox = { name: "inbox", read: (api, _, signal) => api("/api/inbox?unrouted=1", signal) };
