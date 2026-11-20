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
  const expected = (/** @type {number | null} */ revision) => (revision == null ? [] : ["--expected", String(revision)]);

  return {
    desk: ({ all = false } = {}) => read("desk", ...(all ? ["--all"] : [])),
    asks: () => read("asks", "--all"),
    tasks: () => read("tasks"),
    // One task with its brief, plan, pull request, runs and asks, as `/api/tasks/{id}` serves it.
    task: (/** @type {Id} */ id) => read("task", String(id)),
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
    // A field left out comes from the factory's [request] table; the reply is the new task's key.
    add: async (/** @type {string} */ text, /** @type {{ workflow?: string, band?: string, lane?: string }} */ { workflow, band, lane } = {}) => (await call(["add", ...inFactory,
      ...(workflow ? ["--workflow", workflow] : []), ...(band ? ["--band", band] : []), ...(lane ? ["--lane", lane] : []), "--", text])).trim(),
    // A revision makes priory refuse the change once the state moved past the read it was made on.
    hold: (/** @type {Id} */ task, /** @type {{ reason: string, until?: string }} */ { reason, until }, /** @type {number | null} */ revision = null) =>
      call(["hold", String(task), "--reason", reason, ...(until ? ["--until", until] : []), ...expected(revision)]),
    release: (/** @type {Id} */ task, /** @type {number | null} */ revision = null) => call(["release", String(task), ...expected(revision)]),
    cancel: (/** @type {Id} */ task, /** @type {number | null} */ revision = null) => call(["cancel", String(task), ...expected(revision)]),
    // Pausing stops new runs from starting; the live ones go on.
    pause: (/** @type {string} */ reason = "") => call(["pause", ...(reason ? [reason] : [])]),
    unpause: () => call(["unpause"]),
    move: (/** @type {Id} */ task, /** @type {{ band?: string, top?: boolean, before?: Id, after?: Id }} */ { band, top, before, after }, /** @type {number | null} */ revision = null) => call(["move", String(task),
      ...(band ? ["--band", band] : []),
      ...(top ? ["--top"] : before != null ? ["--before", String(before)] : after != null ? ["--after", String(after)] : []),
      ...expected(revision)]),
    // The shell command line that attaches a tmux client to the task's live run, quoted by priory.
    attach: async (/** @type {Id} */ task) => (await call(["attach", String(task), "--print"])).trim(),
  };
}
