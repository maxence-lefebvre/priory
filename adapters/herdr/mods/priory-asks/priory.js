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
    // The rules `autolink` applies, as `GET /api/links` serves them.
    links: () => read("links", ...inFactory),
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
    // A comment starts a thread on a row, by the row's own id; reply, resolve and reopen name a comment.
    comment: (/** @type {string} */ row, /** @type {string} */ text, /** @type {number} */ revision) => call(["report", "comment", row, "--expected", String(revision), text]),
    reply: (/** @type {Id} */ comment, /** @type {string} */ text) => call(["report", "reply", String(comment), text]),
    resolve: (/** @type {Id} */ comment) => call(["report", "resolve", String(comment)]),
    reopen: (/** @type {Id} */ comment) => call(["report", "reopen", String(comment)]),
    seen: ({ at = /** @type {string | null} */ (null) } = {}) => call(["seen", ...(at ? ["--at", at] : [])]),
    resume: (/** @type {Id} */ task) => call(["resume", String(task)]),
    revive: (/** @type {Id} */ task) => call(["revive", String(task)]),
    todoDone: (/** @type {Id} */ todo) => call(["todo", "done", String(todo)]),
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

// The web view's autolinks, in crates/priory-core/web/links.js; a test keeps the two the same.
/**
 * Splits `text` into spans, each `{ text }` or `{ text, url }`. At each point the earliest match
 * wins, and the first rule on a tie. A rule whose URL is not `https:`, whose regex this engine
 * refuses, or that needs `{repo}` when `repo` is null, links nothing.
 * @param {string} text @param {{ pattern: string, url: string }[]} links @param {string | null} repo
 * @returns {{ text: string, url?: string }[]}
 */
export function autolink(text, links, repo) {
  const rules = links.flatMap(({ pattern, url }) => {
    if (!url.startsWith("https://") || (url.includes("{repo}") && !repo)) return [];
    try {
      return [{ regex: new RegExp(pattern, "gu"), url }];
    } catch {
      return [];
    }
  });
  const spans = [];
  let at = 0;
  for (;;) {
    let best = null;
    for (const rule of rules) {
      rule.regex.lastIndex = at;
      let match = rule.regex.exec(text);
      while (match && !match[0] && match.index < text.length) {
        rule.regex.lastIndex = match.index + (text.codePointAt(match.index) > 0xffff ? 2 : 1);
        match = rule.regex.exec(text);
      }
      if (match?.[0] && (!best || match.index < best.match.index)) best = { rule, match };
    }
    if (!best) break;
    const { rule, match } = best;
    if (match.index > at) spans.push({ text: text.slice(at, match.index) });
    const url = rule.url
      .replaceAll("{repo}", () => String(repo).split("/").map(encodeURIComponent).join("/"))
      .replace(/\$(\d+)/g, (_, group) => encodeURIComponent(match[Number(group)] ?? ""));
    spans.push({ text: match[0], url });
    at = match.index + match[0].length;
  }
  if (at < text.length) spans.push({ text: text.slice(at) });
  return spans;
}

const ENTITIES = /** @type {Record<string, string>} */ ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" });
const escapeHtml = (/** @type {string} */ text) => text.replace(/[&<>"']/g, (c) => ENTITIES[c]);

/**
 * `autolink` spans as HTML for Herdr's `TextView.html`, which opens a link with the system's browser.
 * @param {{ text: string, url?: string }[]} spans
 */
export const spansHtml = (spans) => spans
  .map(({ text, url }) => {
    const html = escapeHtml(text).replaceAll("\n", "<br>");
    return url ? `<a href="${escapeHtml(url)}">${html}</a>` : html;
  })
  .join("");

/**
 * A task's `stack` for the views; one pull request keeps the single PR look. Only `latest`, the
 * task's `pull_request`, has known checks.
 * @param {any[] | null | undefined} stack
 * @param {any} [latest]
 * @returns {{ label: string | null, entries: { number: number, url: string, branch: string, base: string, state: string, next: boolean, checks: string | null }[] }}
 */
export function prStack(stack, latest = null) {
  if (!stack || stack.length < 2) return { label: null, entries: [] };
  const next = stack.findIndex((pull) => pull.state === "open");
  const checks = latest?.checks ?? [];
  const failing = checks.filter((/** @type {any} */ check) => !["success", "skipped", "neutral"].includes(check.conclusion)).length;
  const known = !checks.length ? null : failing ? `${failing} of ${checks.length} checks failing` : `${checks.length} checks pass`;
  return {
    label: `${stack.length} PRs`,
    entries: stack.map((pull, at) => ({
      number: pull.number,
      url: `https://github.com/${String(pull.pull_request).replace(/^github:/, "").replace(/#\d+$/, "")}/pull/${pull.number}`,
      branch: pull.branch,
      base: pull.base,
      state: pull.state,
      next: at === next,
      checks: pull.number === latest?.number ? known : null,
    })),
  };
}
