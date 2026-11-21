// A run's terminal: its screen read from /api/runs/{id}/screen while the pane is shown (docs/daemon/serve.md).

const POLL_MS = 1000;
const CALM_MS = 3000;
// Unchanged replies in a row before polling slows to CALM_MS.
const CALM_AFTER = 10;
// Rows of scrollback a "Load earlier" tap adds, and the server's cap.
const EARLIER_STEP = 500;
export const EARLIER_CAP = 2000;

/**
 * Polls the screen of the run `show` names, one read at a time, while it is shown. It keeps the
 * screen in memory only. `read(run, have, history)` resolves to the reply or rejects with an error that may
 * carry the HTTP `status`; `changed` runs when the state a page draws changed.
 * @param {{ read: (run: number, have: string | null, history: number) => Promise<any>, changed?: () => void,
 *   timer?: typeof setTimeout, clear?: typeof clearTimeout, now?: () => number }} io
 */
export function screenWatch({ read, changed = () => {}, timer = setTimeout, clear = clearTimeout, now = Date.now }) {
  let run = null, shown = false, flying = false, handle = null, unchanged = 0;
  // A new run or a sign-out starts a new generation; a reply for an older one is dropped.
  // `again` asks for a read as soon as the one in flight ends.
  let generation = 0, again = false;
  // `at` is when the screen was read; `stale` that the last read failed; `gone` that the run has no screen any more.
  let screen = null, at = null, stale = false, gone = false;
  // Rows of scrollback to ask for, and how many the held screen was asked with.
  let history = 0, held = 0;

  const later = () => {
    clear(handle);
    handle = run !== null && shown && !gone ? timer(poll, unchanged >= CALM_AFTER ? CALM_MS : POLL_MS) : null;
  };

  async function poll() {
    if (run === null || !shown || gone || flying) return;
    const asked = generation;
    clear(handle);
    handle = null;
    flying = true;
    again = false;
    try {
      const rows = history;
      const reply = await read(run, held === rows ? screen?.hash ?? null : null, rows);
      if (asked !== generation) return;
      unchanged = reply.same ? unchanged + 1 : 0;
      if (!reply.same) [screen, held] = [reply, rows];
      // The hash leaves scrollback out, so an unchanged screen still says how much grew above it.
      else if (reply.scrollback !== undefined) screen = { ...screen, scrollback: reply.scrollback };
      at = now();
      stale = false;
    } catch (error) {
      if (asked !== generation) return;
      if (error.status === 401) forget();
      else if (error.status === 404) gone = true;
      else stale = true;
    } finally {
      flying = false;
      if (asked !== generation || again) poll();
      else later();
      changed();
    }
  }

  function forget() {
    generation++;
    run = screen = at = null;
  }

  return {
    /** Shows run `next`'s screen, and polls it from now on. */
    show(next) {
      if (next !== run) {
        forget();
        run = next;
        unchanged = history = held = 0;
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
      again = flying;
      poll();
    },
    /** Signed out: forget the screen. */
    stop() {
      this.hide();
      forget();
    },
    /** Reads `EARLIER_STEP` more rows of scrollback, now, up to the server's cap. */
    earlier() {
      history = Math.min(history + EARLIER_STEP, EARLIER_CAP);
      this.revision();
    },
    // `more`: tmux keeps rows above those shown, and the cap allows reading them.
    state: () => ({ run, screen, at, stale, gone, more: Boolean(screen && screen.scrollback > screen.earlier && history < EARLIER_CAP) }),
  };
}

/**
 * Says `text` to task `task`'s agent through `POST /api/tasks/{task}/say`, the outbox's way: typed
 * once its run is idle, never as a slash command. Null for a blank line; else whether it was said,
 * and the note the pane shows, or `signOut` for a refused token.
 * @param {(path: string, body: object) => Promise<{ status: number, body: any }>} post
 */
export async function sayLine(post, task, text) {
  const line = text.trim();
  if (!line) return null;
  let status = 0, body = {};
  try {
    ({ status, body } = await post(`/api/tasks/${task}/say`, { text: line }));
  } catch {
    // The connection dropped: the daemon may have the line.
  }
  if (status === 401) return { said: false, signOut: true };
  if (status === 200) return { said: true, note: `Said: ${body.ok}. The agent reads it once its run is idle.` };
  const note = status === 503 ? "Could not say: no priory daemon runs beside this server. Start priory daemon, then send again."
    : status === 504 || status === 0 ? "The daemon may have queued this but did not confirm it. Check the task before you send again."
      : `Could not say: ${body?.error || `the server answered ${status}`}.`;
  return { said: false, note };
}

// xterm's eight colours, to which a 256-palette or RGB colour falls by distance. A colour that is
// not grey is brightened first, so a dark diff background keeps its hue instead of turning black.
const EIGHT = [[0, 0, 0], [205, 0, 0], [0, 205, 0], [205, 205, 0], [0, 0, 238], [205, 0, 205], [0, 205, 205], [229, 229, 229]];
const nearest = (colour) => {
  const [low, high] = [Math.min(...colour), Math.max(...colour)];
  return closest(high - low > 24 ? colour.map((value) => (value * 255) / high) : colour);
};
const closest = (rgb) => EIGHT.reduce((best, colour, at) => {
  const distance = colour.reduce((sum, value, i) => sum + (value - rgb[i]) ** 2, 0);
  return distance < best[1] ? [at, distance] : best;
}, [0, Infinity])[0];
const CUBE = [0, 95, 135, 175, 215, 255];
const palette = (n) => (n < 8 ? n : n < 16 ? n - 8 : n < 232
  ? nearest([CUBE[Math.floor((n - 16) / 36)], CUBE[Math.floor((n - 16) / 6) % 6], CUBE[(n - 16) % 6]])
  : nearest(Array(3).fill(8 + 10 * (n - 232))));
const FLAGS = { 1: "bo", 2: "fa", 3: "it", 4: "ul", 7: "rv" };
const OFF = { 22: ["bo", "fa"], 23: ["it"], 24: ["ul"], 27: ["rv"] };

// A colour after 38 or 48 in `params`, from `at`: its index of the eight and how many params it took.
function extended(params, at) {
  const rest = params[at].split(":").slice(1);
  const colon = rest.length > 0;
  const parts = colon ? rest : params.slice(at + 1);
  const byte = (text) => (/^\d{1,3}$/.test(text ?? "") && Number(text) < 256 ? Number(text) : null);
  if (parts[0] === "5") return [byte(parts[1]) === null ? null : palette(byte(parts[1])), colon ? 0 : 2];
  if (parts[0] !== "2") return [null, 0];
  // The colon form may carry a colour space id before the three values.
  const rgb = (colon && parts.length > 4 ? parts.slice(2, 5) : parts.slice(1, 4)).map(byte);
  return [rgb.length === 3 && !rgb.includes(null) ? nearest(rgb) : null, colon ? 0 : 4];
}

// The style after one SGR code's `params`; codes this page does not draw change nothing.
function apply(style, params) {
  const list = params.split(";");
  for (let at = 0; at < list.length; at++) {
    const code = Number(list[at].split(":")[0] || 0);
    if (code === 0) style = { flags: new Set(), fg: null, bg: null };
    else if (code === 4 && list[at] === "4:0") style.flags.delete("ul");
    else if (FLAGS[code]) style.flags.add(FLAGS[code]);
    else if (OFF[code]) OFF[code].forEach((flag) => style.flags.delete(flag));
    else if (code >= 30 && code <= 37) style.fg = code - 30;
    else if (code >= 90 && code <= 97) style.fg = code - 90;
    else if (code >= 40 && code <= 47) style.bg = code - 40;
    else if (code >= 100 && code <= 107) style.bg = code - 100;
    else if (code === 39) style.fg = null;
    else if (code === 49) style.bg = null;
    else if (code === 38 || code === 48) {
      const [colour, used] = extended(list, at);
      if (colour !== null) style[code === 38 ? "fg" : "bg"] = colour;
      at += used;
    }
  }
  return style;
}

// Reverse video fills the cell with the text's colour, `r0` to `r7`, or with the default text
// colour, `rv`; the background it had is dropped.
function classes({ flags, fg, bg }) {
  const names = ["bo", "fa", "it", "ul"].filter((flag) => flags.has(flag));
  if (flags.has("rv")) return names.concat(fg === null ? "rv" : `r${fg}`).join(" ");
  return names.concat(fg === null ? [] : [`f${fg}`], bg === null ? [] : [`b${bg}`]).join(" ");
}

/**
 * The screen's rows as parts of text, each with the classes of its SGR style and whether it is a
 * `[redacted]` the pane draws dim. The style carries from row to row, as tmux writes it. Only SGR
 * codes are read; any other escape stays text.
 * @param {string[]} lines
 */
export function screenRows(lines) {
  let style = { flags: new Set(), fg: null, bg: null };
  return lines.map((line) => {
    const parts = [];
    line.split(/\x1b\[([0-9;:]*)m/).forEach((piece, at) => {
      if (at % 2) return void (style = apply(style, piece));
      const cls = classes(style);
      for (const text of piece.split(/(\[redacted\])/)) if (text) parts.push({ text, cls, hidden: text === "[redacted]" });
    });
    return parts;
  });
}
