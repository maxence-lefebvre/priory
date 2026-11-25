// The app's pages, by name: entries/index.js lists them, and each has its own file there. This module
// imports none of them, so any module may read the list while it runs.

/**
 * A page's entry. `name` is the first segment of its address.
 * - In React: `List` beside a drawer or other panels, `Page` alone, `Detail` of one ask, task or batch,
 *   `Drawer`. Each takes no prop but `Detail` and `Drawer`, which take the `id` an address names, and
 *   `Drawer` the `panels` beside it.
 * - Legacy, the same four in lower case, each a function that answers the HTML: `list(wide)`,
 *   `page(size)`, `detail(id)`, `drawer(id, wide, panels)`. `needs` are the listings `list` and `page`
 *   draw from, `detailNeeds` those of `detail` and `drawer`, read by nobody once a part is in React.
 * - Document events: `click(action, target, event)`, `submit(action, form)`, `input(target)`,
 *   `change(target)`, `key(event)`, and `keyFirst(event)`, which also runs inside a field. True: acted.
 * - Legacy draws: `fields` are the ids of text fields that keep their caret, `kept` the cached reads
 *   `state.data` holds, `older` whether the page shows a listing's older rows, `sheet` redraws a
 *   sheet, `before` and `after(host)` run around a draw, `drain(element)` answers the armed action of a
 *   countdown, `live(desk)` hears each live read of the
 *   desk, `landed` runs once fresh listings land, `left` as the last legacy page leaves, and
 *   `signedOut` while the token still works.
 */
export const entries = [];
export const register = (list) => void entries.splice(0, entries.length, ...list);
export const entryOf = (name) => entries.find((entry) => entry.name === name);

/** The first truthy answer of the entries to `hook`. Every entry before it ran. */
export const first = (hook, ...args) => entries.some((entry) => entry[hook]?.(...args));
/** Runs `hook` of every entry. */
export const each = (hook, ...args) => entries.forEach((entry) => entry[hook]?.(...args));
