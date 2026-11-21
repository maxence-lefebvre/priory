// A run's terminal: its screen read from /api/runs/{id}/screen while the pane is shown (docs/daemon/serve.md).

const POLL_MS = 1000;
const CALM_MS = 3000;
// Unchanged replies in a row before polling slows to CALM_MS.
const CALM_AFTER = 10;

/**
 * Polls the screen of the run `show` names, one read at a time, while it is shown. It keeps the
 * screen in memory only. `read(run, have)` resolves to the reply or rejects with an error that may
 * carry the HTTP `status`; `changed` runs when the state a page draws changed.
 * @param {{ read: (run: number, have: string | null) => Promise<any>, changed?: () => void,
 *   timer?: typeof setTimeout, clear?: typeof clearTimeout, now?: () => number }} io
 */
export function screenWatch({ read, changed = () => {}, timer = setTimeout, clear = clearTimeout, now = Date.now }) {
  let run = null, shown = false, flying = false, handle = null, unchanged = 0;
  // `at` is when the screen was read; `stale` that the last read failed; `gone` that the run has no screen any more.
  let screen = null, at = null, stale = false, gone = false;

  const later = () => {
    clear(handle);
    handle = run !== null && shown && !gone ? timer(poll, unchanged >= CALM_AFTER ? CALM_MS : POLL_MS) : null;
  };

  async function poll() {
    if (run === null || !shown || gone || flying) return;
    const asked = run;
    clear(handle);
    handle = null;
    flying = true;
    try {
      const reply = await read(asked, screen?.hash ?? null);
      if (asked !== run) return;
      unchanged = reply.same ? unchanged + 1 : 0;
      if (!reply.same) screen = reply;
      at = now();
      stale = false;
    } catch (error) {
      if (asked !== run) return;
      if (error.status === 401) run = screen = at = null;
      else if (error.status === 404) gone = true;
      else stale = true;
    } finally {
      flying = false;
      if (asked === run) later();
      else poll();
      changed();
    }
  }

  return {
    /** Shows run `next`'s screen, and polls it from now on. */
    show(next) {
      if (next !== run) {
        run = next;
        screen = at = null;
        unchanged = 0;
        stale = gone = false;
      }
      if (shown && (flying || handle)) return;
      shown = true;
      poll();
    },
    /** Stops polling until the pane shows again. */
    hide() {
      shown = false;
      clear(handle);
      handle = null;
    },
    /** The state moved: read again now. */
    revision() {
      unchanged = 0;
      poll();
    },
    /** Signed out: forget the screen. */
    stop() {
      this.hide();
      run = screen = at = null;
    },
    state: () => ({ run, screen, at, stale, gone }),
  };
}

/** A row as its plain parts and its `[redacted]` parts, which the pane draws dim. */
export const rowParts = (/** @type {string} */ line) =>
  line.split(/(\[redacted\])/).filter(Boolean).map((text) => ({ text, hidden: text === "[redacted]" }));
