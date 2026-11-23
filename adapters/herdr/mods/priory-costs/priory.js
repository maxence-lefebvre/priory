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
    // Every run with its task, outcome, model and tokens, newest first, as `/api/runs?all=1` serves them.
    runs: () => read("runs"),
    // The events that reached no task, grouped by the resource's last owner, as `/api/inbox?unrouted=1` serves them.
    unrouted: () => read("inbox", "--unrouted"),
    // Sends the resource's next event to the task, unless the state moved past `revision`.
    subscribe: (/** @type {Id} */ task, /** @type {string} */ resource, /** @type {number | null} */ revision = null) =>
      call(["subscribe", String(task), resource, ...expected(revision)]),
    // Ends the task's subscription; what the task owns otherwise, such as its key, stays.
    unsubscribe: (/** @type {Id} */ task, /** @type {string} */ resource) => call(["unsubscribe", String(task), resource]),
    // Marks the task wrong with a URL that shows it, for the risk report, or clears every mark.
    label: (/** @type {Id} */ task, /** @type {{ evidence?: string, happened?: string, clear?: boolean }} */ { evidence, happened, clear = false }) =>
      call(["label", ...inFactory, String(task), "wrong", ...(clear ? ["--clear"] : ["--evidence", String(evidence), ...(happened ? ["--happened", happened] : [])])]),
    // One task with its brief, plan, pull request, runs and asks, as `/api/tasks/{id}` serves it.
    task: (/** @type {Id} */ id) => read("task", String(id)),
    queue: () => read("queue", ...inFactory),
    schedules: () => read("schedules", ...inFactory),
    // The rules `autolink` applies, as `GET /api/links` serves them.
    links: () => read("links", ...inFactory),
    report: (/** @type {Id | null} */ task = null) => read("report", "show", ...(task == null ? [] : [String(task)])),
    reports: () => read("report", "list"),
    // This week's cost against last week's, split by workflow, agent, model, machine or repo; prices come from the factory.
    costs: (/** @type {string | null} */ by = null) => read("costs", ...inFactory, ...(by ? ["--by", by] : [])),
    attention: () => read("report", "attention"),
    // Each workflow's steps and edges, with the irreversible steps and the paths to one with no human decision.
    flows: () => read("flows", ...inFactory),
    // The views as text, since the Priory bar hashes exactly what `priory views --json` prints.
    views: () => call(["views", ...inFactory, "--json"]),
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
    // The memory candidates that wait for a person, each with the memory of its name runs get now.
    memories: () => read("memories", ...inFactory),
    // A person's verdict on a candidate, published on the factory's log; every runner applies it.
    acceptMemory: (/** @type {string} */ id) => call(["memories", "accept", ...inFactory, id]),
    dropMemory: (/** @type {string} */ id) => call(["memories", "drop", ...inFactory, id]),
    // Marks a report read, so its row leaves the desk.
    read: (/** @type {Id} */ seq) => call(["inbox", "read", String(seq)]),
    // A field left out comes from the factory's [request] table; the reply is the new task's key.
    add: async (/** @type {string} */ text, /** @type {{ workflow?: string, band?: string, lane?: string }} */ { workflow, band, lane } = {}) => (await call(["add", ...inFactory,
      ...(workflow ? ["--workflow", workflow] : []), ...(band ? ["--band", band] : []), ...(lane ? ["--lane", lane] : []), "--", text])).trim(),
    // A revision makes priory refuse the change once the state moved past the read it was made on.
    hold: (/** @type {Id} */ task, /** @type {{ reason: string, until?: string }} */ { reason, until }, /** @type {number | null} */ revision = null) =>
      call(["hold", String(task), "--reason", reason, ...(until ? ["--until", until] : []), ...expected(revision)]),
    release: (/** @type {Id} */ task, /** @type {number | null} */ revision = null) => call(["release", String(task), ...expected(revision)]),
    cancel: (/** @type {Id} */ task, /** @type {number | null} */ revision = null) => call(["cancel", String(task), ...expected(revision)]),
    // Resume, cancel or release another runner's task, named `runner:key`, at the generation the panel read.
    remote: (/** @type {string} */ act, /** @type {string} */ target, /** @type {number} */ generation) => call([act, "--generation", String(generation), target]),
    // Pausing stops new runs from starting; the live ones go on.
    pause: (/** @type {string} */ reason = "") => call(["pause", ...(reason ? [reason] : [])]),
    unpause: () => call(["unpause"]),
    move: (/** @type {Id} */ task, /** @type {{ band?: string, top?: boolean, before?: Id, after?: Id }} */ { band, top, before, after }, /** @type {number | null} */ revision = null) => call(["move", String(task),
      ...(band ? ["--band", band] : []),
      ...(top ? ["--top"] : before != null ? ["--before", String(before)] : after != null ? ["--after", String(after)] : []),
      ...expected(revision)]),
    // The shell command line that attaches a tmux client to the task's live run, quoted by priory.
    // A read-only client watches: it cannot type, and leaves the agent's window size alone.
    attach: async (/** @type {Id} */ task, { readOnly = false } = {}) => (await call(["attach", String(task), ...(readOnly ? ["--read-only"] : []), "--print"])).trim(),
  };
}

/**
 * A panel's read loop: reads, waits with `priory wait` until the state moves past the revision it
 * read, and reads again, until `stopped()`. `read` returns the state revision it read, or null to
 * wait from the revision the last wait returned. A failed read or wait calls `failed`, then backs
 * off `retry` ms before it reads again.
 * @param {{ read: () => Promise<number | null>, wait: (since: number) => Promise<number>, sleep: (ms: number) => Promise<unknown>,
 *   failed: (error: unknown) => void, retry: number, stopped?: () => boolean }} loop
 */
export async function follow({ read, wait, sleep, failed, retry, stopped = () => false }) {
  // -1 makes a first wait with no revision return at once with the current one.
  let since = -1;
  while (!stopped()) {
    try {
      const revision = await read();
      if (stopped()) return;
      since = await wait(revision ?? since);
    } catch (error) {
      if (stopped()) return;
      failed(error);
      await sleep(retry);
    }
  }
}

// The web view's autolinks, in crates/priory-core/web/links.js; a test keeps the two the same.
/**
 * Splits `text` into spans, each `{ text }` or `{ text, url }`. At each point the earliest match
 * wins, and the first rule on a tie. A bare `https://` URL links to itself and wins a tie, so no rule
 * links a part of it. A rule whose URL is not `https:`, whose regex this engine refuses, or that
 * needs `{repo}` when `repo` is null, links nothing.
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
  // A bare URL ends at a space, `<`, `>` or `"`, and leaves out the punctuation, `…` and unbalanced
  // `)` that prose puts after it. Its link is its own text, when that parses as an `https:` URL.
  const bare = (/** @type {number} */ from) => {
    const regex = /https:\/\/[^\s<>"]+/gu;
    regex.lastIndex = from;
    for (let match = regex.exec(text); match; match = regex.exec(text)) {
      let url = match[0];
      for (let last = ""; last !== url;) {
        last = url;
        url = url.replace(/[.,;:!?'…]+$/u, "");
        if (url.endsWith(")") && url.split(")").length > url.split("(").length) url = url.slice(0, -1);
      }
      try {
        if (new URL(url).protocol === "https:") return { index: match.index, text: url, url };
      } catch {
        // Not a URL: look for the next one.
      }
    }
    return null;
  };
  const spans = [];
  let at = 0;
  for (;;) {
    let best = bare(at);
    for (const rule of rules) {
      rule.regex.lastIndex = at;
      let match = rule.regex.exec(text);
      while (match && !match[0] && match.index < text.length) {
        rule.regex.lastIndex = match.index + (text.codePointAt(match.index) > 0xffff ? 2 : 1);
        match = rule.regex.exec(text);
      }
      if (!match?.[0] || (best && match.index >= best.index)) continue;
      const found = match;
      const url = rule.url
        .replaceAll("{repo}", () => String(repo).split("/").map(encodeURIComponent).join("/"))
        .replace(/\$(\d+)/g, (_, group) => encodeURIComponent(found[Number(group)] ?? ""));
      best = { index: match.index, text: match[0], url };
    }
    if (!best) break;
    if (best.index > at) spans.push({ text: text.slice(at, best.index) });
    spans.push({ text: best.text, url: best.url });
    at = best.index + best.text.length;
  }
  if (at < text.length) spans.push({ text: text.slice(at) });
  return spans;
}

const ENTITIES = /** @type {Record<string, string>} */ ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" });
const escapeHtml = (/** @type {string} */ text) => text.replace(/[&<>"']/g, (c) => ENTITIES[c]);

/**
 * `autolink` spans as HTML for Herdr's `TextView.html`, which opens a link with the system's browser.
 * It drops `<br>`, so each line is a paragraph.
 * @param {{ text: string, url?: string }[]} spans
 */
export function spansHtml(spans) {
  const lines = [""];
  for (const { text, url } of spans) {
    text.split("\n").forEach((part, index) => {
      if (index > 0) lines.push("");
      if (part) lines[lines.length - 1] += url ? `<a href="${escapeHtml(url)}">${escapeHtml(part)}</a>` : escapeHtml(part);
    });
  }
  return lines.map((line) => `<p>${line}</p>`).join("");
}

// DESIGN.md's Markdown subset, as crates/priory-core/web/markdown.js parses it, drawn as the HTML Herdr's
// TextView takes: no <br>, no class, and a check list's box as a character.
const LISTS = [{ tag: "ul", item: /^\s*[-*+]\s+(.*)$/s }, { tag: "ol", item: /^\s*\d+[.)]\s+(.*)$/s }];
const BOX = /^\[([ xX])\]\s+(.*)$/s;
const FENCE = /^\s*```/;
const HEADING = /^ {0,3}(#{1,6})[ \t]+(.*)$/s;
// A heading's closing hashes, scanned by hand: a regex for them backtracks on long trailing space.
const unclosed = (/** @type {string} */ text) => {
  const trimmed = text.trimEnd();
  let end = trimmed.length;
  while (end && trimmed[end - 1] === "#") end -= 1;
  return end < trimmed.length && /\s/.test(trimmed[end - 1] ?? "") ? trimmed.slice(0, end).trimEnd() : trimmed;
};
const URL_PART = String.raw`https:\/\/(?:[^\s()]|\([^\s()]*\))+`;
// A label holds no `[`, so unmatched brackets scan in linear time. Each bold token is atomic: a whole
// code span or link when one forms, else one character.
const INLINE = new RegExp(
  String.raw`\x60(?<code>[^\x60]+)\x60|\*\*(?<bold>(?:(?=(?<token>\x60[^\x60]+\x60|\[[^[\]]*\]\(${URL_PART}\)|[^*]))\k<token>)+)\*\*|(?<image>!?)\[(?<label>[^[\]]*)\]\((?<url>${URL_PART})\)`,
  "g",
);
const startsBlock = (/** @type {string} */ line) => FENCE.test(line) || HEADING.test(line) || LISTS.some(({ item }) => item.test(line));

/**
 * Splits Markdown into blocks: `{ tag, lines }`, where `tag` is "p", "h3" to "h5", "ul", "ol" or
 * "pre", and `lines` holds a paragraph's lines, a heading, a list's items or a code block's lines.
 * @param {string} text
 */
function parse(text) {
  const lines = String(text ?? "").replace(/\r\n?/g, "\n").split("\n");
  /** @type {{ tag: string, lines: string[] }[]} */
  const out = [];
  for (let at = 0; at < lines.length;) {
    const line = lines[at];
    if (!line.trim()) {
      at += 1;
      continue;
    }
    if (FENCE.test(line)) {
      const code = [];
      for (at += 1; at < lines.length && !FENCE.test(lines[at]); at += 1) code.push(lines[at]);
      out.push({ tag: "pre", lines: code });
      at += 1;
      continue;
    }
    const heading = line.match(HEADING);
    if (heading) {
      out.push({ tag: `h${Math.min(heading[1].length, 3) + 2}`, lines: [unclosed(heading[2])] });
      at += 1;
      continue;
    }
    const list = LISTS.find(({ item }) => item.test(line));
    const para = [];
    if (list) {
      for (; at < lines.length && list.item.test(lines[at]); at += 1) para.push(/** @type {RegExpMatchArray} */ (lines[at].match(list.item))[1]);
      const boxes = para.map((item) => item.match(BOX));
      const checks = list.tag === "ul" && boxes.every(Boolean);
      out.push({ tag: list.tag, lines: checks ? boxes.map((box) => `${box?.[1] === " " ? "☐" : "☑"} ${box?.[2]}`) : para });
      continue;
    }
    for (; at < lines.length && lines[at].trim() && !startsBlock(lines[at]); at += 1) para.push(lines[at]);
    out.push({ tag: "p", lines: para });
  }
  return out;
}

// Herdr stops a render that runs too long, and a panel draws the same agent text on every render.
const REMEMBERED = 2000;
/** @type {WeakMap<object, Map<string, string>>} each links list's drawn Markdown, by repo and text */
const drawnHtml = new WeakMap();
/** @type {Map<string, string>} */
const drawnText = new Map();
/** @type {Map<string, ReturnType<typeof parse>>} */
const parsed = new Map();

/**
 * `map`'s value for `key`, computed once; past `REMEMBERED` keys the oldest goes.
 * @template T @param {Map<string, T>} map @param {string} key @param {() => T} compute
 */
function remembered(map, key, compute) {
  const known = map.get(key);
  if (known !== undefined) return known;
  const value = compute();
  if (map.size >= REMEMBERED) map.delete(/** @type {string} */ (map.keys().next().value));
  map.set(key, value);
  return value;
}

/** `parse` once per text, as a panel's HTML and its label both draw it. @param {string} text */
const blocks = (text) => remembered(parsed, String(text ?? ""), () => parse(text));

/**
 * Markdown as HTML for Herdr's `TextView.html`: DESIGN.md's subset, every other character escaped, with
 * the factory's autolinks. A link or image whose target is not `https:` shows as its source; an image
 * shows its alt text and loads nothing.
 * @param {string} text @param {{ pattern: string, url: string }[]} links @param {string | null} repo
 */
export function markdownHtml(text, links, repo) {
  if (!drawnHtml.has(links)) drawnHtml.set(links, new Map());
  const drawn = /** @type {Map<string, string>} */ (drawnHtml.get(links));
  return remembered(drawn, `${repo ?? ""}\n${text ?? ""}`, () => parsedHtml(text, links, repo));
}

/** @param {string} text @param {{ pattern: string, url: string }[]} links @param {string | null} repo */
function parsedHtml(text, links, repo) {
  const plain = (/** @type {string} */ part) => spansHtml(autolink(part, links, repo)).slice(3, -4);
  const inline = (/** @type {string} */ line) => {
    let html = "";
    let at = 0;
    for (const match of line.matchAll(INLINE)) {
      const { code, bold, image, label, url } = /** @type {Record<string, string>} */ (match.groups);
      html += plain(line.slice(at, match.index));
      if (code) html += `<code>${escapeHtml(code)}</code>`;
      else if (bold) html += `<strong>${inline(bold)}</strong>`;
      else html += image ? escapeHtml(label) : `<a href="${escapeHtml(url)}">${escapeHtml(label)}</a>`;
      at = match.index + match[0].length;
    }
    return html + plain(line.slice(at));
  };
  return blocks(text).map(({ tag, lines }) => {
    if (tag === "pre") return `<pre><code>${escapeHtml(lines.join("\n"))}</code></pre>`;
    if (tag === "p") return lines.map((line) => `<p>${inline(line)}</p>`).join("");
    if (tag === "ul" || tag === "ol") return `<${tag}>${lines.map((item) => `<li>${inline(item)}</li>`).join("")}</${tag}>`;
    return `<${tag}>${inline(lines[0])}</${tag}>`;
  }).join("");
}

/** The words Markdown shows, on one line, for a label that cannot hold its links. @param {string} text */
export function markdownText(text) {
  return remembered(drawnText, String(text ?? ""), () => parsedText(text));
}

/** @param {string} text */
function parsedText(text) {
  const inline = (/** @type {string} */ line) => line.replace(INLINE, (_match, code, bold, _token, _image, label) => code ?? (bold ? inline(bold) : label));
  return blocks(text).flatMap(({ tag, lines }) => (tag === "pre" ? lines : lines.map(inline))).join(" ");
}

/** A count and its noun: "1 task", "2 tasks"; `many` covers irregular plurals. @param {number} count @param {string} one @param {string} [many] */
export const plural = (count, one, many = `${one}s`) => `${count} ${count === 1 ? one : many}`;

/** "with <lead>" for an open ask or desk item routed to a lead, else null. @param {any} ask */
export const routedMark = (ask) => (ask?.routed_to && (ask.state ?? "open") === "open" ? `with ${ask.routed_to}` : null);

/** What an open ask's detail says of its lead, else null. @param {any} ask */
export const routedNote = (ask) => (routedMark(ask) ? `${ask.routed_to} is answering. You can still answer first: the first answer stands.` : null);

/** The runner of another runner's ask, which alone lists `submissions`, else null. @param {any} ask */
export const runnerOf = (ask) => (Array.isArray(ask?.submissions) ? ask.runner : null);

// How long a runner may be quiet before its asks say when it was last seen: alive messages come once a minute.
const AWAY_MS = 3 * 60 * 1000;

/**
 * "last seen <age> ago" for another runner's ask whose runner is away, else null.
 * @param {any} ask @param {number} now @param {(iso: string) => string} ago
 */
export function seenAgo(ask, now, ago) {
  if (!runnerOf(ask)) return null;
  if (!ask.last_seen) return "never seen";
  return now - Date.parse(ask.last_seen) > AWAY_MS ? `last seen ${ago(ask.last_seen)} ago` : null;
}

/**
 * What became of the latest answer or dismiss sent to another runner's ask, while it waits or once
 * refused, else null: an applied one shows in the ask itself. @param {any} ask
 */
export function sentNote(ask) {
  const runner = runnerOf(ask);
  const last = runner ? ask.submissions.at(-1) : null;
  if (!last) return null;
  const what = last.verb === "dismiss" ? "the dismiss" : last.text;
  if (last.outcome === "refused") return { text: `${runner} refused ${what}: ${last.why}`, refused: true };
  if (last.outcome) return null;
  return { text: last.verb === "dismiss" ? `Dismiss sent to ${runner}` : `Sent to ${runner}: ${last.text}`, refused: false };
}

const SUBMITTED = { answered: "answered", dismissed: "withdrawn", refused: "refused" };

/** Each answer and dismiss sent to another runner's ask as an answer history row, oldest first. @param {any} ask */
export function submitted(ask) {
  const runner = runnerOf(ask);
  if (!runner) return [];
  return ask.submissions.map((/** @type {any} */ sent) => {
    const outcome = sent.outcome === "refused" ? `${runner} refused it: ${sent.why}` : sent.outcome ? `${runner} applied it` : "Not applied yet";
    const what = sent.verb === "dismiss" ? "the dismiss" : sent.text;
    return { tile: SUBMITTED[sent.outcome] ?? "queued", text: `Sent ${what} to ${runner} for revision ${sent.revision}. ${outcome}`, at: sent.sent };
  });
}

/** Who answered, as the views say it: a person, `agent:<lead>`, auto, or `session <name>` for a person's own session. Null when unrecorded. @param {string | null | undefined} by */
export const answeredBy = (by) => (!by ? null : by.startsWith("agent:") ? by : by === "auto" ? "auto" : by.startsWith("session:") ? `session ${by.slice(8)}` : "a person");

const date = (/** @type {string} */ iso) => new Date(iso).toLocaleString("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23" });

/** As the web view's age(): "just now", "12 min", "4 h", then a date (DESIGN.md, Copy). @param {string} iso @param {number} now */
export function age(iso, now) {
  const elapsed = now - Date.parse(iso);
  if (elapsed < 60_000) return "just now";
  const minutes = Math.round(elapsed / 60_000);
  if (minutes < 60) return `${minutes} min`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)} h`;
  return date(iso);
}

/** As the web view's ago(): "just now", "12 min ago", or "on Nov 20, 08:00" past a day. @param {string} iso @param {number} now */
export function ago(iso, now) {
  const text = age(iso, now);
  if (text === "just now") return text;
  return / (min|h)$/.test(text) ? `${text} ago` : `on ${text}`;
}

// Copied from crates/priory-core/web/inbox.js; a test keeps the two the same.
const SOURCES = { github: "GitHub", linear: "Linear", slack: "Slack", factory: "" };

/** An event's type in plain words: `linear.issue.updated` reads "Linear issue updated". */
export function eventWords(type) {
  const [source, ...rest] = type.split(".");
  return [SOURCES[source] ?? source, ...rest].filter(Boolean).join(" ").replaceAll("_", " ");
}

/** A Since you looked line, `<type> <subject>: <outcome>`, with its type in plain words; a report's line as it is. @param {string} what */
export function sinceWords(what) {
  const [type] = what.split(" ", 1);
  return /^[a-z_]+(\.[a-z_]+)+$/.test(type) ? eventWords(type) + what.slice(type.length) : what;
}

/** The longest heading of an untitled ask, in characters, before its ellipsis. */
export const HEADING_MAX = 80;

/**
 * An ask's heading, where its text is a heading or one line: its title; else its question when that
 * is one line of at most 80 characters; else one derived from its task's key and step, or from the
 * question's first sentence when it has no task, cut at a word to about 80 characters. The detail
 * shows the full question in its body. @param {any} ask
 */
export function askHeading(ask) {
  if (ask?.title) return ask.title;
  const question = String(ask?.question ?? "").trim();
  if (question.length <= HEADING_MAX && !question.includes("\n")) return question;
  const key = String(ask?.key ?? "").replace(/^(linear|github|slack|cli):/, "");
  if (key && ask?.step) return cutAt(`${key} asks at ${ask.step}`, HEADING_MAX);
  const end = question.search(/[.?!](\s|$)|\n/);
  const first = end < 0 ? question : question.slice(0, question[end] === "\n" ? end : end + 1).trim();
  const step = ask?.step && !first.startsWith(ask.step) ? `${ask.step}: ` : "";
  return cutAt(`${step}${first}`, HEADING_MAX);
}

/** `text` cut at the last word that ends within `max` characters, with an ellipsis. @param {string} text @param {number} max */
export function cutAt(text, max) {
  if (text.length <= max) return text;
  const space = text.slice(0, max + 1).lastIndexOf(" ");
  const kept = space > max / 2 ? text.slice(0, space) : text.slice(0, max);
  return `${kept.replace(/[\s.,;:!?-]+$/, "")}…`;
}

/** The full question, for the detail's body when its heading does not show all of it, else null. @param {any} ask */
export const askBody = (ask) => (ask?.question && askHeading(ask) !== ask.question ? ask.question : null);

/** The tile a check draws by its conclusion: one with none has not finished (DESIGN.md, Pull request summary). @param {string | null} conclusion */
export function checkState(conclusion) {
  if (!conclusion) return "running";
  if (conclusion === "success") return "passed";
  return ["skipped", "neutral"].includes(conclusion) ? "skipped" : "failed";
}

/** A review or check state in plain words: `changes_requested` reads "changes requested", `success` "passed". @param {string | null} state */
export const stateWords = (state) => (state === "success" ? "passed" : String(state ?? "").replaceAll("_", " "));

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
  const failing = checks.filter((/** @type {any} */ check) => check.conclusion && !["success", "skipped", "neutral"].includes(check.conclusion)).length;
  const running = checks.filter((/** @type {any} */ check) => !check.conclusion).length;
  const known = !checks.length ? null
    : failing ? `${failing} of ${plural(checks.length, "check")} failing`
    : running ? `${running} of ${plural(checks.length, "check")} running`
    : plural(checks.length, "check passes", "checks pass");
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
