// The week's attention.
export const attention = () => ({ queryKey: ["attention"], meta: { kept: true, read: (api, signal) => api("/api/attention", signal) } });
