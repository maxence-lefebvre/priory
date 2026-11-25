// The factory's workflows. The server reads the factory once, so a page reads them once.
export const flowsKey = () => ["flows"];
export const flows = { name: "flows", kept: true, once: true, read: (api, _, signal) => api("/api/flows", signal) };
