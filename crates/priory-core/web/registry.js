// The app's pages, by name: entries/index.js lists them, and each has its own file there. This module
// imports none of them, so any module may read the list while it runs.

/**
 * A page's entry. `name` is the first segment of its address. `List` draws beside a drawer or other
 * panels, `Page` alone, `Detail` one ask, task or batch, and `Drawer` an ask beside a list. Each takes
 * no prop but `Detail` and `Drawer`, which take the `id` an address names, and `Drawer` the `panels`
 * beside it. `key(event)` acts on a shortcut of the page; true: it acted.
 */
export const entries = [];
export const register = (list) => void entries.splice(0, entries.length, ...list);
export const entryOf = (name) => entries.find((entry) => entry.name === name);

const REACT = { list: "List", page: "Page", detail: "Detail", drawer: "Drawer" };
export const reactPart = ({ kind, name }) => entryOf(name)?.[REACT[kind]];

/** The first truthy answer of the entries to `hook`. Every entry before it ran. */
export const first = (hook, ...args) => entries.some((entry) => entry[hook]?.(...args));
