// The week's spend, split by workflow or another part.
export const costs = (by) => ({ queryKey: ["costs", by], meta: { kept: true, read: (api, signal) => api(`/api/costs?by=${by}`, signal) } });
