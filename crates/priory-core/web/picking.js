// Picks waiting out their Undo, then in flight, one per row. A pick wakes the report's agent, so
// nothing leaves before its Undo time passes, and a pick never waits on another row's.
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
    put(entry.row, { ...entry, sending: true });
    try {
      await send(entry);
    } finally {
      put(entry.row, null);
    }
  };
  return {
    /** Starts `pick`'s Undo, unless its row already has a pick pending. */
    start(pick) {
      if (rows.has(pick.row)) return false;
      const entry = { ...pick, sending: false };
      entry.timer = setTimeout(() => go(entry), UNDO_MS);
      put(pick.row, entry);
      return true;
    },
    /** Cancels `row`'s pick while it waits out its Undo; true when there was one. */
    undo(row) {
      const entry = rows.get(row);
      if (!entry || entry.sending) return false;
      clearTimeout(entry.timer);
      put(row, null);
      return true;
    },
    /** Cancels every pick still waiting whose `keep` is false, and returns them. */
    cancel(keep = () => false) {
      const gone = [...rows.values()].filter((entry) => !entry.sending && !keep(entry));
      for (const entry of gone) {
        clearTimeout(entry.timer);
        rows.delete(entry.row);
      }
      if (gone.length) changed(new Map(rows));
      return gone;
    },
  };
}
