// The factory's workflows. The server reads the factory once, so a page reads them once.
export const flows = () => ({ queryKey: ["flows"], meta: { kept: true, once: true, read: (api, signal) => api("/api/flows", signal) } });
