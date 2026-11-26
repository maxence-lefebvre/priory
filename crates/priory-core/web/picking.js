// Picks waiting out their Undo, then in flight, one per row. A pick wakes the report's agent, so
// nothing leaves before its Undo time passes, and a pick never waits on another row's. A batch, such
// as a section's Pick all recommended, shares one Undo.
export const UNDO_MS = 4000;

/**
 * @param {{ send: (pick: object) => Promise<unknown>, changed: (rows: Map<string, object>) => void }} hooks
 * `send` posts one pick; `changed` gets every row's pending pick after each change.
 */
export function picking({ send, changed }) {
  const rows = new Map();
  const put = (row, entry) => {
    if (entry) rows.set(row, entry);
    else rows.delete(row);
    changed(new Map(rows));
  };
  const go = async (entry) => {
    // A pick that Undo or a new read took out of its batch stays.
    if (rows.get(entry.row) !== entry) return;
    put(entry.row, { ...entry, sending: true });
    try {
      await send(entry);
    } finally {
      put(entry.row, null);
    }
  };
  const drop = (entries) => {
    for (const entry of entries) rows.delete(entry.row);
    if (entries.length) changed(new Map(rows));
    return entries;
  };
  return {
    /** Starts `pick`'s Undo, unless its row already has a pick pending. */
    start(pick) {
      return this.batch(null, [pick]) === 1;
    },
    /**
     * Starts one Undo for every pick of `picks` whose row has none pending, as batch `key`; a null
     * key is a single pick. Returns how many it started.
     */
    batch(key, picks) {
      const entries = picks.filter((pick) => !rows.has(pick.row)).map((pick) => ({ ...pick, batch: key, sending: false, timer: null }));
      if (!entries.length) return 0;
      const timer = setTimeout(() => entries.forEach(go), UNDO_MS);
      for (const entry of entries) {
        entry.timer = timer;
        rows.set(entry.row, entry);
      }
      changed(new Map(rows));
      return entries.length;
    },
    /** Cancels `row`'s pick, and every other pick of its batch, while it waits out its Undo. */
    undo(row) {
      const entry = rows.get(row);
      if (!entry || entry.sending) return false;
      clearTimeout(entry.timer);
      drop([...rows.values()].filter((each) => each.timer === entry.timer && !each.sending));
      return true;
    },
    /** Cancels every pick still waiting whose `keep` is false, and returns them. */
    cancel(keep = () => false) {
      return drop([...rows.values()].filter((entry) => !entry.sending && !keep(entry)));
    },
  };
}
