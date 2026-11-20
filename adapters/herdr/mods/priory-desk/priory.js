// Reads and changes a Priory factory through the priory CLI, for Herdr mods.
// Every call runs the binary with an argv array: no shell, no parsing of text output.

/**
 * @typedef {(command: string, args: string[], options: { on?: string | null, timeout?: number }) =>
 *   Promise<{ code: number, stdout: string, stderr: string }>} Exec Herdr's `exec`.
 * @typedef {string | number} Id
 */

/**
 * @param {{ exec: Exec, binary?: string, on?: string | null, factory?: string | null }} options
 *   `on` is the Herdr machine that runs priory; `factory` the folder `queue` and `schedules` read.
 */
export function priory({ exec, binary = "priory-oss", on = null, factory = null }) {
  /** @param {string[]} args @param {number} [timeout] */
  async function call(args, timeout) {
    const result = await exec(binary, args, { on, timeout });
    if (result.code !== 0) throw new Error(result.stderr.trim().split("\n").pop() || `priory ${args[0]}: exit ${result.code}`);
    return result.stdout;
  }
  /** @param {string[]} args */
  const read = async (...args) => JSON.parse(await call([...args, "--json"]));
  const inFactory = factory ? [factory] : [];

  return {
    desk: ({ all = false } = {}) => read("desk", ...(all ? ["--all"] : [])),
    asks: () => read("asks", "--all"),
    tasks: () => read("tasks"),
    queue: () => read("queue", ...inFactory),
    schedules: () => read("schedules", ...inFactory),
    report: (/** @type {Id | null} */ task = null) => read("report", "show", ...(task == null ? [] : [String(task)])),
    // priory wait gives up after 25 s with the same revision; Herdr's exec would stop it at 30 s.
    wait: async (/** @type {number} */ since) => Number(await call(["wait", "--since", String(since)], 60_000)),
    // `run` confirms an answer that runs the ask's command, as the person's second press does.
    answer: (/** @type {Id} */ ask, /** @type {string} */ text, /** @type {number} */ revision, { run = false } = {}) =>
      call(["answer", "--expected", String(revision), ...(run ? ["--run"] : []), "--", String(ask), text]),
    dismiss: (/** @type {Id} */ ask, /** @type {number} */ revision) => call(["dismiss", "--expected", String(revision), String(ask)]),
    snooze: (/** @type {string} */ kind, /** @type {Id} */ id, /** @type {{ for?: string, until?: string } | null} */ when) => call(["snooze", kind, String(id), ...(
      when?.for ? ["--for", when.for] : when?.until ? ["--until", when.until] : ["--clear"])]),
    say: (/** @type {Id} */ task, /** @type {string} */ text) => call(["say", String(task), text]),
    pick: (/** @type {string} */ pick, /** @type {string} */ option, /** @type {number} */ revision) => call(["report", "pick", pick, option, "--expected", String(revision)]),
    seen: ({ at = /** @type {string | null} */ (null) } = {}) => call(["seen", ...(at ? ["--at", at] : [])]),
    resume: (/** @type {Id} */ task) => call(["resume", String(task)]),
    revive: (/** @type {Id} */ task) => call(["revive", String(task)]),
    hold: (/** @type {Id} */ task, /** @type {{ reason: string, until?: string }} */ { reason, until }) => call(["hold", String(task), "--reason", reason, ...(until ? ["--until", until] : [])]),
    release: (/** @type {Id} */ task) => call(["release", String(task)]),
    cancel: (/** @type {Id} */ task) => call(["cancel", String(task)]),
    // Pausing stops new runs from starting; the live ones go on.
    pause: (/** @type {string} */ reason = "") => call(["pause", ...(reason ? [reason] : [])]),
    unpause: () => call(["unpause"]),
    move: (/** @type {Id} */ task, /** @type {{ band?: string, top?: boolean, before?: Id, after?: Id }} */ { band, top, before, after }) => call(["move", String(task),
      ...(band ? ["--band", band] : []),
      ...(top ? ["--top"] : before != null ? ["--before", String(before)] : after != null ? ["--after", String(after)] : [])]),
    // The shell command line that attaches a tmux client to the task's live run, quoted by priory.
    attach: async (/** @type {Id} */ task) => (await call(["attach", String(task), "--print"])).trim(),
  };
}
