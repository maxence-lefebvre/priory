// The Asks panel's view model: `priory asks --all --json` in, rows and states out. No drawing, no IO.

import { age, ago, answeredBy, autolink, checkState, stateWords, prStack, routedMark, routedNote, runnerOf, seenAgo, sentNote, submitted } from "./priory.js";

/** @typedef {{ pattern: string, url: string }[]} Links the rules `priory links` lists */

/** @param {any} ask @param {number} now */
const snoozed = (ask, now) => ask.state === "open" && !ask.back_early && Date.parse(ask.snoozed_until ?? "") > now;
const sameDay = (/** @type {string} */ iso, /** @type {number} */ now) => new Date(iso).toDateString() === new Date(now).toDateString();
const clock = (/** @type {string} */ iso) => new Date(iso).toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
const date = (/** @type {string} */ iso) => new Date(iso).toLocaleString("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
/** A time of day today, else a date. @param {string} iso @param {number} now */
const when = (iso, now) => (sameDay(iso, now) ? clock(iso) : date(iso));

const sentence = (/** @type {string} */ text) => text.charAt(0).toUpperCase() + text.slice(1);

/** @param {any} ask @param {number} now */
function row(ask, now) {
  return {
    id: ask.id,
    tile: ask.state === "open" ? "ask" : ask.state,
    key: ask.key,
    step: ask.step,
    age: ask.state === "answered" && ask.answered_at ? when(ask.answered_at, now) : age(ask.since, now),
    command: Boolean(ask.command),
    text: ask.title || ask.question,
    meta: ask.state === "answered" ? ["Answered", ask.answer]
      : ask.state === "withdrawn" ? ["Withdrawn", ask.withdrawn_why]
      : ask.recommendation ? ["Recommends", ask.recommendation] : null,
    snoozed: snoozed(ask, now) ? `Snoozed until ${when(ask.snoozed_until, now)}` : null,
    backEarly: ask.state === "open" && ask.back_early ? `Back early: ${ask.back_early.reason} ${ask.revision}` : null,
    ...(routedMark(ask) && { routed: routedMark(ask) }),
    ...(runnerOf(ask) && { runner: runnerOf(ask), away: seenAgo(ask, now, (iso) => age(iso, now)), sent: sentNote(ask) }),
    ...(ask.state === "answered" && answeredBy(ask.answered_by) && { by: answeredBy(ask.answered_by) }),
  };
}

/** The Risk facet's values in the order it offers them, as `priory asks --risk` takes them. */
const RISKS = ["irreversible", "red", "yellow", "green", "none"];

/** Each value of a field, in first-seen order, with how many asks carry it. @param {any[]} asks @param {string} field */
function counted(asks, field) {
  const counts = new Map();
  for (const ask of asks) if (ask[field] != null) counts.set(ask[field], (counts.get(ask[field]) ?? 0) + 1);
  return [...counts];
}

/**
 * The saved views with their counts, and the chosen view's sections, each left out when empty.
 * @param {any[]} all
 * @param {{ view: string, now: number, bands?: string[], facets?: { command?: boolean, lane?: string, band?: string, risk?: string, task?: string, runner?: string } }} options
 *   `bands` is the factory's, highest first; an ask without a band shows under `none`, and one
 *   without a risk under the risk `none`.
 */
export function asksList(all, { view, now, bands = [], facets = {} }) {
  const band = (/** @type {any} */ ask) => ask.band ?? "none";
  const risks = (/** @type {any} */ ask) => (ask.risk?.length ? ask.risk : ["none"]);
  const asks = all.filter((ask) => (!facets.command || ask.command) && (!facets.lane || ask.lane === facets.lane)
    && (!facets.band || band(ask) === facets.band) && (!facets.risk || risks(ask).includes(facets.risk))
    && (!facets.task || ask.key === facets.task) && (!facets.runner || ask.runner === facets.runner));
  // `since` may carry any offset, and other runners' asks come last, so sort by instant as the web's askGroups.
  const unanswered = asks.filter((ask) => ask.state === "open").sort((a, b) => (Date.parse(a.since) || 0) - (Date.parse(b.since) || 0));
  const open = unanswered.filter((ask) => !snoozed(ask, now));
  const asleep = unanswered.filter((ask) => snoozed(ask, now));
  const backToday = open.filter((ask) => ask.back_early && sameDay(ask.back_early.at, now));
  const answered = asks.filter((ask) => ask.state === "answered").sort((a, b) => (b.answered_at ?? "").localeCompare(a.answered_at ?? ""));
  const answeredToday = answered.filter((ask) => ask.answered_at && sameDay(ask.answered_at, now));
  const answeredEarlier = answered.filter((ask) => !answeredToday.includes(ask));
  const withdrawn = asks.filter((ask) => ask.state === "withdrawn").reverse();
  const views = [
    { id: "open", label: "Open", count: open.length },
    { id: "snoozed", label: "Snoozed", count: asleep.length },
    { id: "answered", label: "Answered", count: answered.length },
    { id: "all", label: "All", count: asks.length },
  ];
  /** @param {string} id @param {string} title @param {any[]} list @param {string} [note] */
  const section = (id, title, list, note = String(list.length)) => ({ id, title, note, rows: list.map((ask) => row(ask, now)) });
  const sections = {
    open: [section("open", "Open", open, `${open.length}, oldest first`)],
    snoozed: [section("snoozed", "Snoozed", asleep), section("back", "Back early today", backToday)],
    answered: [section("today", "Answered today", answeredToday), section("earlier", "Answered earlier", answeredEarlier)],
    all: [
      section("open", "Open", unanswered, `${unanswered.length}, oldest first`),
      section("today", "Answered today", answeredToday),
      section("earlier", "Answered earlier", answeredEarlier),
      section("withdrawn", "Withdrawn", withdrawn),
    ],
  }[view] ?? [];
  const order = [...bands, "none"];
  const byBand = counted(all.map((ask) => ({ band: band(ask) })), "band").sort(([a], [b]) => order.indexOf(a) - order.indexOf(b));
  const byRisk = counted(all.flatMap((ask) => risks(ask).map((risk) => ({ risk }))), "risk").sort(([a], [b]) => RISKS.indexOf(a) - RISKS.indexOf(b));
  return { views, facets: { lane: counted(all, "lane"), band: byBand, risk: byRisk, task: counted(all, "key"), runner: counted(all, "runner") }, sections:sections.filter((candidate) => candidate.rows.length) };
}

/** An agent's command runs on the options in its `runs`; an outside ask's string command never runs. @param {any} ask @param {string} option */
export const runsCommand = (ask, option) => Array.isArray(ask.command?.runs) && ask.command.runs.includes(option);

/** @type {Record<string, string>} */
const RAN = {
  running: "The command runs.",
  done: "The command ran: exit 0.",
  failed: "The command failed",
  uncertain: "Priory stopped while this ran. Check the target before you act on it.",
};

/** @param {any} ask @returns {{ title: string, words: string[], note: string } | null} */
function command(ask) {
  const owner = runnerOf(ask);
  if (!ask.command) return null;
  if (typeof ask.command === "string") {
    return { title: "Command the asker attached", words: [ask.command], note: "Priory stores this command and never runs it: the ask came from outside Priory." };
  }
  const { argv, dir, target, runs } = ask.command;
  return { title: "Command an answer runs", words: argv, note: `In ${dir} (${target}). ${owner ? `Runs on ${owner} when` : "Runs when"} answered ${runs.join(" or ")}. Bound to revision ${ask.revision}. Runs at most once.` };
}

export const ARM_MS = 4000;

/** @typedef {{ id: string | number, revision: number, option: number, at: number }} Armed */

/** Whether `armed` still arms an option of this revision of the ask. @param {Armed | null} armed @param {any} ask @param {number} now */
const holds = (armed, ask, now) => armed !== null && armed.id === ask.id && armed.revision === ask.revision && now - armed.at < ARM_MS;

/**
 * What a press on option `index` does: answer at once, or, for an option that runs a command, arm it
 * first and run it on a second press within ARM_MS (R6.8).
 * @param {any} ask @param {number} index @param {Armed | null} armed @param {number} now
 */
export function press(ask, index, armed, now) {
  const text = ask.options[index];
  if (!runsCommand(ask, text)) return { answer: { text, run: false } };
  if (holds(armed, ask, now) && armed.option === index) return { answer: { text, run: true } };
  return { arm: { id: ask.id, revision: ask.revision, option: index, at: now } };
}

/** " by <who>" for the revision the ask's `answered_by` belongs to: the current one. @param {any} ask @param {any} revision */
const byLine = (ask, revision) => {
  const by = ask.state === "answered" && revision.revision === ask.revision ? answeredBy(ask.answered_by) : null;
  return by ? ` by ${by}` : "";
};

/** Every revision with its answer, every refused answer and submission, and a withdrawal, newest first. @param {any} ask */
function history(ask) {
  // An ask listed before Priory kept its revisions shows only its current one.
  const revisions = ask.revisions ?? [{ revision: ask.revision, at: ask.since, answer: ask.answer, answered_at: ask.answered_at }];
  const dated = [
    ...revisions.flatMap((/** @type {any} */ revision) => [
      { tile: "ask", text: `Asked revision ${revision.revision}${revision.question ? `: ${revision.question}` : ""}`, at: revision.at },
      ...(revision.answer == null ? [] : [{ tile: "answered", text: `Answered ${revision.answer}${byLine(ask, revision)}`, at: revision.answered_at ?? revision.at }]),
    ]),
    ...(ask.refused ?? []).map((/** @type {any} */ refusal) => ({
      tile: "refused", text: `Refused: ${refusal.answer}, sent for revision ${refusal.revision} from ${refusal.by}: ${refusal.why}`, at: refusal.at,
    })),
    ...submitted(ask),
  // Built oldest first; reversed, entries of the same second stay newest first through the stable sort.
  ].reverse().sort((a, b) => b.at.localeCompare(a.at));
  const withdrawn = ask.state === "withdrawn" ? [{ tile: "withdrawn", text: `Withdrawn${ask.withdrawn_why ? `: ${ask.withdrawn_why}` : ""}`, at: "" }] : [];
  return [...withdrawn, ...dated].map(({ tile, text, at }) => ({ tile, text, time: at ? clock(at) : "" }));
}

const minutes = (/** @type {string} */ from, /** @type {string} */ to) => Math.round((Date.parse(to) - Date.parse(from)) / 60_000);

/** The plan step's latest result, else the progress the agent last published. @param {any} task */
function plan(task) {
  if (task.plan) return { text: task.plan.summary, from: `${task.plan.step}, run ${task.plan.run}` };
  const progress = task.progress;
  if (!progress) return null;
  const lines = [
    progress.goal && `Goal: ${progress.goal}`,
    ...(progress.acceptanceCriteria ?? []).map((/** @type {any} */ criterion) => `- ${criterion.id}: ${criterion.description}`),
    progress.latestChange && `Latest change: ${progress.latestChange}`,
    progress.nextAction && `Next: ${progress.nextAction}`,
  ];
  return { text: lines.filter(Boolean).join("\n"), from: "progress" };
}

/** The task's latest pull request, then its whole stack when it has two or more. @param {any} pull @param {any[]} stack */
function pullRequest(pull, stack) {
  const { label, entries } = prStack(stack, pull);
  // The summary needs the pull request's events; the stack does not.
  if (!pull) return label ? { title: label, line: null, checks: [], review: null, prs: label, stack: entries } : null;
  return {
    title: `#${pull.number} ${pull.title ?? ""}`.trim(),
    line: `${pull.branch} into ${pull.base}, head ${String(pull.head_sha).slice(0, 7)}, ${pull.state}`,
    checks: pull.checks.map((/** @type {any} */ check) => ({ name: check.name, state: checkState(check.conclusion), text: check.conclusion ? stateWords(check.conclusion) : "running" })),
    review: pull.review ? stateWords(pull.review) : null,
    ...(label ? { prs: label, stack: entries } : {}),
  };
}

/** The task's runs on a time axis: step, run, span, outcome and tokens. @param {any[]} runs @param {number} now */
function trace(runs, now) {
  return runs.map((run) => {
    const end = run.ended ?? new Date(now).toISOString();
    const tokens = run.tokens ? Object.values(run.tokens).reduce((sum, count) => sum + (Number(count) || 0), 0) : null;
    return {
      tile: run.ended ? run.end ?? "moved" : "ask",
      label: `${run.step}, run ${run.id}`,
      span: `${clock(run.started)} to ${run.ended ? clock(run.ended) : "now"}, ${minutes(run.started, end)} min`,
      outcome: run.outcome,
      tokens: tokens === null ? null : `${tokens} tokens`,
    };
  });
}

/**
 * One ask's detail: the decision first, then what to read. `task` is `priory task ID --json` of the
 * ask's task, when it was read: it fills Brief, Plan, PR and Trace.
 * @param {any} ask
 * @param {{ now: number, armed: Armed | null, task?: any, links?: Links }} options
 */
export function askDetail(ask, { now, armed, task = null, links = [] }) {
  const open = ask.state === "open";
  /** @type {string[]} */
  const options = open ? ask.options ?? [] : [];
  const arming = holds(armed, ask, now) ? armed.option : null;
  const recommended = Math.max(0, options.indexOf(ask.recommendation));
  const request = task?.brief?.request;
  const groomed = task?.brief?.result;
  const brief = request || groomed ? { request, text: groomed?.summary ?? null, from: groomed ? `${groomed.step}, run ${groomed.run}` : null } : null;
  const planned = task ? plan(task) : null;
  const pr = pullRequest(task?.pull_request, task?.stack);
  const runs = trace(task?.runs ?? [], now);
  const question = ask.title || ask.question;
  const why = ask.text || ask.question;
  const owner = runnerOf(ask);
  const away = seenAgo(ask, now, (iso) => age(iso, now));
  return {
    context: [ask.key, ask.step, ask.lane, ask.command ? "Command" : null, owner && [owner, away].filter(Boolean).join(", "), age(ask.since, now), `revision ${ask.revision}`].filter(Boolean),
    question,
    questionSpans: autolink(question, links, ask.repo ?? null),
    riskLine: ask.risk_estimate?.line ?? null,
    recommendation: ask.recommendation ? { text: `Recommended: ${ask.recommendation}.`, next: ask.next_action } : null,
    options: options.map((/** @type {string} */ label, /** @type {number} */ index) => ({
      index,
      label,
      key: index < 9 ? String(index + 1) : "",
      recommended: label === ask.recommendation,
      notes: [label === ask.recommendation && "Recommended", runsCommand(ask, label) && "Runs a command"].filter(Boolean).join(". "),
      armed: arming === index,
      dimmed: arming !== null && arming !== index,
    })),
    chips: /** @type {[string, string][]} */ ([
      ["why", "Why"], brief && ["brief", "Brief"], planned && ["plan", "Plan"], pr && ["pr", "PR"],
      runs.length > 0 && ["trace", "Trace"], ["history", "History"], ask.command && ["command", "Command"],
    ].filter(Boolean)),
    brief,
    plan: planned,
    pr,
    trace: runs,
    why: { text: why, textSpans: autolink(why, links, ask.repo ?? null), run: ask.run, tradeoff: ask.tradeoff, whatWaits: ask.what_waits },
    ...(routedNote(ask) && { routed: routedNote(ask) }),
    ...(owner && {
      // A closed ask still says why its runner refused an answer sent to it.
      sent: open || sentNote(ask)?.refused ? sentNote(ask) : null,
      elsewhere: `This ask belongs to ${owner}. Your answer goes there through the log, and ${owner} applies it when it reads it. Snooze stays on ${owner}.`,
    }),
    decided: ask.state === "answered" ? `Answered: ${ask.answer}.${ask.answered_at ? ` ${sentence(ago(ask.answered_at, now))}.` : ""}${answeredBy(ask.answered_by) ? ` By ${answeredBy(ask.answered_by)}.` : ""}`
      : ask.state === "withdrawn" ? `Withdrawn${ask.withdrawn_why ? `: ${ask.withdrawn_why}` : ""}.` : null,
    ran: ask.ran ? { text: ask.ran.status === "failed" ? `${RAN.failed}: ${ask.ran.error}.` : RAN[ask.ran.status] ?? ask.ran.status, warn: ask.ran.status === "uncertain" } : null,
    history: history(ask),
    command: command(ask),
    bar: options.length ? { count: `${options.length} options`, option: recommended } : null,
    canSnooze: open && ask.task != null && !owner,
    // A step's or a hold's ask takes only its options: no free text and no dismissal.
    takesText: open && ask.free_text !== false,
  };
}

/**
 * Why an answer or dismissal did not go, given the ask as read, the ask as Priory lists it now, and
 * the CLI's error line.
 * @param {any} sent @param {any} fresh @param {string} error @param {number} now
 */
export function refusal(sent, fresh, error, now) {
  if (fresh?.state === "answered") return `Already answered ${ago(fresh.answered_at, now)}: ${fresh.answer}.`;
  if (fresh?.state === "withdrawn") return `Withdrawn: ${fresh.withdrawn_why}.`;
  if (fresh && fresh.revision !== sent.revision) return `The question changed. Read revision ${fresh.revision} and answer again.`;
  return `Not sent: ${error}. The draft stays here.`;
}

/** `hour`:00 local time tomorrow, for `priory snooze --until`. @param {number} hour @param {number} now */
export function tomorrowAt(hour, now) {
  const at = new Date(now);
  at.setDate(at.getDate() + 1);
  at.setHours(hour, 0, 0, 0);
  return at.toISOString();
}
