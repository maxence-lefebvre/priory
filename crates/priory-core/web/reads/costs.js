// The week's spend, split by workflow or another part, and the week's attention.
export const costsKey = (by) => ["costs", by];
export const attentionKey = () => ["attention"];
export const costs = { name: "costs", kept: true, read: (api, [, by], signal) => api(`/api/costs?by=${by}`, signal) };
export const attention = { name: "attention", kept: true, read: (api, _, signal) => api("/api/attention", signal) };
