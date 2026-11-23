import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dossier, nextPick, refusal } from "../mods/priory-bonjour/view.js";
import { find } from "./kit.js";

process.env.TZ = "UTC";

// Captured from `priory report show --json` on a scratch factory: its report stored with
// `priory report put`, then its picks chosen and applied by SQL as the daemon and the workflow
// would. report-revised.json is the same state after someone picked Later on FND-2413.
const fixture = (name) => JSON.parse(readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8"));
const read = fixture("report.json");
const now = Date.parse("2026-11-20T08:30:00Z");

test("draws the report's sections in its own order, each with its count or its number of rows", () => {
  const view = dossier(read, { now });
  assert.deepEqual(view.sections.map((section) => [section.id, section.title, section.count]), [
    ["reading", "Reading", 2],
    ["slack", "Slack notes", 1],
    ["sentry", "Sentry", 3],
    ["agree", "To agree", 2],
    ["blocked", "Blocked", 2],
    ["moved", "Moved", 2],
  ]);
});

test("a row shows its tile, key, text, meta, owner and time, and agent text stays as written", () => {
  const sections = dossier(read, { now }).sections;
  const rows = (id) => sections.find((section) => section.id === id).rows.map(({ pick, target, threads, textSpans, ...row }) => row);
  assert.deepEqual(rows("slack"), [{
    id: '["slack",0]', tile: null, key: null, owner: null, time: null, meta: null,
    text: "Yesterday: merged the MCP v2 plan review, answered 4 Priory asks.\nToday: agree the picks and land #212. <b>not html</b>",
  }]);
  assert.deepEqual(rows("blocked"), [
    { id: '["blocked",0]', tile: "blocked", key: "PRI-55", text: "Permission prompt denied", meta: "resume or edit the agent", owner: "on Max", time: null },
    { id: '["blocked",1]', tile: "held", key: "PRI-69", text: "Held until #212 merges", meta: "then it releases", owner: "on #212", time: null },
  ]);
  // A time today shows its clock, an earlier one its date.
  assert.deepEqual(rows("moved").map((row) => [row.id, row.tile, row.time]), [['["moved",0]', "done", "Nov 19, 18:42"], ['["moved",1]', "dropped", "06:05"]]);
  // Row ids are unique only in their section, so the section names them; a row without one goes by its
  // place, which no row's own id can equal.
  assert.deepEqual(rows("sentry").map((row) => row.id), ['["sentry","fnd-2409"]', '["sentry","fnd-2413"]']);
});

test("an open pick offers its options with the recommended one marked, and a chosen pick folds to its choice", () => {
  const picks = Object.fromEntries(dossier(read, { now }).sections.flatMap((section) => section.rows).filter((row) => row.pick).map((row) => [row.id, row.pick]));
  assert.deepEqual(picks['["sentry","fnd-2413"]'], {
    id: "2026-11-20_sentry_fnd-2413",
    state: "open",
    options: [
      { label: "Take", key: "1", recommended: false },
      { label: "Later", key: "2", recommended: true },
      { label: "Drop", key: "3", recommended: false },
    ],
    note: "Recommends Later",
    chosen: null,
  });
  assert.equal(picks['["agree",1]'].note, null);
  assert.deepEqual(picks['["agree",0]'], {
    id: "2026-11-20_agree_priory-schedules", state: "agreed", options: picks['["sentry","fnd-2413"]'].options.map((option) => ({ ...option, recommended: option.label === "Take" })),
    chosen: "Take", note: "Waiting for the workflow to apply it.",
  });
  assert.deepEqual(picks['["sentry","fnd-2409"]'], {
    id: "2026-11-20_sentry_fnd-2409", state: "applied", options: [], chosen: "Take", note: "requested task cli:FND-2409",
  });
  assert.equal(Object.keys(picks).length, 4);
});

test("the toolbar and greeting give the date, how many picks are agreed, and both revisions", () => {
  const view = dossier(read, { now });
  assert.deepEqual(
    { date: view.date, greeting: view.greeting, since: view.since, progress: view.progress, task: view.task, revision: view.revision, waited: view.waited },
    {
      date: "Nov 20",
      greeting: "Bonjour, Max",
      since: "Since Thu 17:00. 2 picks to agree.",
      progress: { agreed: 2, total: 4, label: "2 of 4 agreed" },
      // A pick takes the report's revision; `priory wait` takes the state's.
      task: 1, revision: 3, waited: 6,
    },
  );
  const allAgreed = fixture("report-revised.json");
  allAgreed.report.sections[3].rows[1].pick.chosen = "Skip";
  assert.equal(dossier(allAgreed, { now }).since, "Since Thu 17:00. Every pick is agreed.");
  // Hand-built: a report since this morning, with no picks.
  const calm = { revision: 9, report: { ...read.report, since: "2026-11-20T06:00:00Z", sections: [read.report.sections[0]] } };
  assert.deepEqual([dossier(calm, { now }).since, dossier(calm, { now }).progress], ["Since 06:00.", { agreed: 0, total: 0, label: "No picks" }]);
});

test("the Picks view keeps only the rows with a pick to agree, in their sections", () => {
  const all = dossier(read, { now });
  assert.deepEqual(all.views, [{ id: "all", label: "All sections", count: null }, { id: "picks", label: "Picks", count: 2 }]);
  const picks = dossier(read, { now, view: "picks" });
  assert.deepEqual(picks.sections.map((section) => [section.title, section.rows.map((row) => row.id)]), [
    ["Sentry", ['["sentry","fnd-2413"]']],
    ["To agree", ['["agree",1]']],
  ]);
  assert.deepEqual(picks.views, all.views);
});

test("a row's own id never takes the place of a row without one", () => {
  // Hand-built: a valid report whose second row is named "1" and whose third row has no id.
  const clash = { revision: 1, report: { ...read.report, sections: [{ id: "s", title: "S", rows: [{ text: "a" }, { id: "1", text: "b" }, { text: "c" }] }] } };
  assert.deepEqual(dossier(clash, { now }).sections[0].rows.map((row) => row.id), ['["s",0]', '["s","1"]', '["s",2]']);
});

test("Next pick goes to the next pick to agree after the selected row, and wraps", () => {
  const view = dossier(read, { now });
  assert.equal(nextPick(view, null), '["sentry","fnd-2413"]');
  assert.equal(nextPick(view, '["sentry","fnd-2413"]'), '["agree",1]');
  assert.equal(nextPick(view, '["agree",1]'), '["sentry","fnd-2413"]');
  // From a row without a pick to agree, the next one below it.
  assert.equal(nextPick(view, '["agree",0]'), '["agree",1]');
  assert.equal(nextPick(view, '["moved",1]'), '["sentry","fnd-2413"]');
  const done = fixture("report-revised.json");
  done.report.sections[3].rows[1].pick.chosen = "Skip";
  assert.equal(nextPick(dossier(done, { now }), null), null);
});

test("a refused pick says why, from the report as Priory shows it after the refusal", () => {
  const sent = { pick: "2026-11-20_sentry_fnd-2413", option: "Take", revision: 3 };
  const error = "priory report pick: task 1's report is at revision 4, not 3";
  assert.equal(refusal(sent, fixture("report-revised.json"), error), "Already picked Later.");
  // Hand-built: the report moved on, by a rewrite that left this pick open, or that dropped it.
  const rewritten = { revision: 8, report: { ...read.report, revision: 5 } };
  assert.equal(refusal(sent, rewritten, error), "The report changed to revision 5. Read it and pick again.");
  const dropped = { revision: 8, report: { ...read.report, revision: 5, sections: [] } };
  assert.equal(refusal(sent, dropped, error), "The report no longer holds this pick.");
  assert.equal(refusal(sent, read, "priory report pick: no priory daemon listens on /run/priory.sock"),
    "Not sent: priory report pick: no priory daemon listens on /run/priory.sock.");
  assert.equal(refusal(sent, null, "priory report show: no task has a report"), "Not sent: priory report show: no task has a report.");
});

test("a report row's text turns the factory's keys into link spans", () => {
  const { links } = fixture("links.json");
  const rows = dossier(read, { now, links }).sections.flatMap((section) => section.rows);
  assert.deepEqual(rows.find((row) => row.key === "PRI-69").textSpans, [{ text: "Held until #212 merges" }]);
  const linked = { ...read, report: { ...read.report, sections: [{ id: "s", title: "S", count: null, rows: [{ text: "Triage ENG-7 today" }] }] } };
  assert.deepEqual(dossier(linked, { now, links }).sections[0].rows[0].textSpans,
    [{ text: "Triage " }, { text: "ENG-7", url: "https://linear.app/acme/issue/ENG-7" }, { text: " today" }]);
});

test("a row shows its comment threads, oldest first, with their replies and state", () => {
  // Hand-built from the threads `priory report show --json` prints after a comment, an agent's reply and a resolve.
  const commented = structuredClone(read);
  commented.report.sections[2].rows[1].comments = [
    { id: 1, author: "person", text: "Check the release branch first", created: "2026-11-20T08:05:00Z", resolved: false,
      replies: [{ id: 2, author: "agent", text: "It is clean <b>there</b>", created: "2026-11-19T21:00:00Z" }] },
    { id: 3, author: "person", text: "Done", created: "2026-11-20T08:20:00Z", resolved: true, replies: [] },
  ];
  const row = dossier(commented, { now }).sections[2].rows[1];
  assert.equal(row.target, "fnd-2413");
  assert.deepEqual(row.threads, [
    { id: 1, author: "person", when: "08:05", text: "Check the release branch first", resolved: false,
      replies: [{ id: 2, author: "agent", when: "Nov 19, 21:00", text: "It is clean <b>there</b>" }] },
    { id: 3, author: "person", when: "08:20", text: "Done", resolved: true, replies: [] },
  ]);
  // A row without an id takes no comments.
  assert.deepEqual([dossier(read, { now }).sections[1].rows[0].target, dossier(read, { now }).sections[1].rows[0].threads], [null, []]);
});

test("a press on a row's link opens the link and leaves the row unselected", async () => {
  const { context, draw, herdr, press, settle } = await import("./kit.js");
  const { default: Bonjour } = await import("../mods/priory-bonjour/main.js");
  herdr.reset();
  herdr.replies = { report: (args) => (args[1] === "list" ? { revision: 6, reports: [] } : read), links: { links: [{ pattern: "OpenTelemetry", url: "https://opentelemetry.io/" }] } };
  const panel = new Bonjour();
  const cx = context();
  panel.init({}, cx);
  await settle();
  const row = dossier(read, { now }).sections[0].rows[0];

  press(draw(panel, cx), `text-${row.id}`);
  assert.equal(panel.selected, null);
  press(draw(panel, cx), `row-${row.id}`);
  assert.equal(panel.selected, row.id);
});

test("the selected row's key, text and meta select and copy; the other rows stay buttons", async () => {
  const { find, panel, view } = await mounted({ report: (/** @type {string[]} */ args) => (args[1] === "list" ? { revision: 6, reports: [] } : read), links: { links: [] } });
  const [first, second] = dossier(read, { now }).sections[0].rows;
  panel.selected = first.id;
  const tree = view();
  assert.notEqual(find(tree, `row-${first.id}`).kind, "button");
  assert.equal(find(tree, `text-${first.id}`).calls.selectable?.[0], true);
  assert.equal(find(tree, `row-${second.id}`).kind, "button");
  assert.equal(find(tree, `text-${second.id}`), null);
});

test("a row draws its Markdown: a link shows its label, linked, selected or not", async () => {
  // A row of Max's report, 2026-11-22: its text is a Markdown link.
  const text = "[Fragments: November 20](https://martinfowler.com/fragments/2026-11-20.html): light reading";
  const linked = { ...read, report: { ...read.report, sections: [{ id: "s", title: "S", count: null, rows: [{ id: "r", text }] }] } };
  const { find, panel, view, text: shown } = await mounted({ report: (/** @type {string[]} */ args) => (args[1] === "list" ? { revision: 6, reports: [] } : linked), links: { links: [] } });
  const id = '["s","r"]';
  for (const selected of [null, id]) {
    panel.selected = selected;
    const words = find(view(), `text-${id}`);
    assert.equal(shown(words), "Fragments: November 20: light reading");
    assert.match(words.html, /<a href="https:\/\/martinfowler\.com\/fragments\/2026-11-20\.html">Fragments: November 20<\/a>/);
  }
});

test("a press on a row's formatted text without a link selects the row, and an image shows its alt text", async () => {
  const rows = [{ id: "bold", text: "**Ship** it" }, { id: "image", text: "![the chart](https://img.test/c.png)" }];
  const formatted = { ...read, report: { ...read.report, sections: [{ id: "s", title: "S", count: null, rows }] } };
  const { find, panel, press, view, text: shown } = await mounted({ report: (/** @type {string[]} */ args) => (args[1] === "list" ? { revision: 6, reports: [] } : formatted), links: { links: [] } });
  assert.equal(shown(find(view(), 'row-["s","image"]')), "the chart");
  press(view(), 'text-["s","bold"]');
  assert.equal(panel.selected, '["s","bold"]');
});

/** The Bonjour panel drawn against `replies`, once its first read settled. @param {Record<string, unknown>} replies */
async function mounted(replies) {
  const kit = await import("./kit.js");
  const { default: Bonjour } = await import("../mods/priory-bonjour/main.js");
  kit.herdr.reset();
  kit.herdr.replies = replies;
  const panel = new Bonjour();
  const cx = kit.context();
  panel.init({}, cx);
  await kit.settle();
  return { ...kit, panel, view: () => kit.draw(panel, cx) };
}

// Hand-built: an older report of task 4, and `priory report list --json` naming both.
const older = { revision: 6, report: { ...structuredClone(read.report), task: 4, date: "2026-11-19", greeting: "Bonjour again" } };
const list = { revision: 6, reports: [{ task: 1, date: "2026-11-20" }, { task: 4, date: "2026-11-19" }] };
const reports = (/** @type {string[]} */ args) => (args[1] === "list" ? list : args[2] === "4" ? older : read);

test("the older and newer arrows show the report written before or after the one shown", async () => {
  const { panel, view, press, find, settle, text, herdr } = await mounted({ report: reports });
  assert.equal(find(view(), "report-newer").calls.disabled?.[0], true);

  // A new thread names its row by id, which the newest report holding it takes, so only the newest
  // report offers Comment.
  assert.ok(find(view(), 'comment-["sentry","fnd-2413"]'));
  press(view(), "report-older");
  await settle();
  assert.equal(find(view(), 'comment-["sentry","fnd-2413"]'), null);

  assert.match(text(view()), /Nov 19/);
  assert.match(text(view()), /Bonjour again/);
  assert.equal(find(view(), "report-older").calls.disabled?.[0], true);
  // The panel keeps reading the report it shows when Priory wakes it.
  herdr.wake();
  await settle();
  assert.deepEqual(herdr.argv.filter((args) => args[1] === "show").at(-1), ["report", "show", "4", "--json"]);
  assert.match(text(view()), /Bonjour again/);

  press(view(), "report-newer");
  await settle();
  assert.match(text(view()), /Nov 20/);
  assert.equal(panel.shown, null);
});

test("search keeps the rows that hold its text and says how many it found", async () => {
  const { panel, view, press, find, text } = await mounted({ report: reports });
  assert.equal(find(view(), "report-query"), null);

  press(view(), "report-search");
  panel.query.set_value("pri-6");

  const drawn = text(view());
  assert.match(drawn, /3 found/);
  assert.match(drawn, /Held until #212 merges/);
  assert.doesNotMatch(drawn, /Private fonts/);
  panel.query.set_value("no such words");
  assert.match(text(view()), /Nothing in this report holds “no such words”\./);
  // Closing the search shows every row again.
  press(view(), "report-search");
  assert.match(text(view()), /Private fonts/);
});

test("Change reopens an agreed pick, and a new option goes through the revision-guarded pick", async () => {
  const { panel, view, press, find, settle, herdr } = await mounted({ report: reports });
  const row = '["agree",0]';
  assert.equal(find(view(), `pick-${row}-2`), null);

  press(view(), `change-${row}`);
  // The chosen option shows solid; picking it again changes nothing.
  assert.equal(find(view(), `pick-${row}-1`).calls.selected?.[0], true);
  press(view(), `pick-${row}-1`);
  assert.equal(panel.pending, null);
  press(view(), `pick-${row}-2`);
  herdr.timers.at(-1).fire();
  await settle();

  assert.deepEqual(herdr.argv.find((args) => args[1] === "pick"), ["report", "pick", "2026-11-20_agree_priory-schedules", "Later", "--expected", "3"]);
});

test("a check marks the chosen option only; the recommendation is the note's words", async () => {
  const { view, press, find, all, text } = await mounted({ report: reports });
  const row = '["sentry","fnd-2413"]';
  const checks = (key) => all(find(view(), `pick-${row}-${key}`), "svg").length;
  assert.equal(checks(2), 0, "the recommended option shows no check before a choice");
  assert.match(text(view()), /Recommends Later/);

  press(view(), `pick-${row}-1`);

  assert.deepEqual([checks(1), checks(2)], [1, 0]);
});

// report-long.json has the shape and lengths of a live 169-row report, 124 rows in Sentry, with every
// word replaced by a synthetic one and Sentry padded to 155 rows, so the report holds 200.
const long = fixture("report-long.json");
const longReplies = { report: (/** @type {string[]} */ args) => (args[1] === "list" ? { revision: 6, reports: [] } : long), links: { links: [] } };
const sentry = () => dossier(long, { now }).sections.find((section) => section.id === "sentry").rows;
/** A row as drawn: a line is a Button, a pick card holds its text. @param {any} tree @param {{ id: string }} row */
const drawnRow = (tree, row) => find(tree, `row-${row.id}`) ?? find(tree, `text-${row.id}`);

/** How many times `draw` splits Markdown into blocks, which only the Markdown parser does with this pattern. @param {() => unknown} draw */
function parses(draw) {
  const replace = String.prototype.replace;
  let count = 0;
  String.prototype.replace = function (pattern, ...rest) {
    if (pattern instanceof RegExp && pattern.source === String.raw`\r\n?`) count += 1;
    return replace.call(this, pattern, ...rest);
  };
  try {
    draw();
  } finally {
    String.prototype.replace = replace;
  }
  return count;
}

test("a long report parses each row's Markdown once, not on every render", async () => {
  const { panel, view } = await mounted(longReplies);
  for (const shown of ["all", "picks"]) {
    panel.view = shown;
    parses(view);
    assert.equal(parses(view), 0, `the ${shown} view parsed again`);
  }
});

test("a row's text and meta parse once each, for its label and its drawn Markdown alike", async () => {
  // Words no other test draws, so no earlier render remembers them.
  const rows = [{ id: "r", text: "**Parse** me once, [linked](https://parse.test/1)", meta: "and `this` once" }];
  const fresh = { ...read, report: { ...read.report, sections: [{ id: "s", title: "S", count: null, rows }] } };
  const { view } = await mounted({ report: (/** @type {string[]} */ args) => (args[1] === "list" ? { revision: 6, reports: [] } : fresh), links: { links: [] } });
  assert.equal(parses(view), 2);
  assert.equal(parses(view), 0);
});

test("a long section folds past its first rows, and Show more draws the rest", async () => {
  const { view, press, find, text } = await mounted(longReplies);
  const rows = sentry();
  assert.ok(drawnRow(view(), rows[0]));
  assert.equal(drawnRow(view(), rows.at(-1)), null);
  const more = find(view(), "more-sentry");
  assert.match(text(more), /^Show 140 more rows$/);

  press(view(), "more-sentry");

  assert.ok(drawnRow(view(), rows.at(-1)));
  assert.equal(find(view(), "more-sentry"), null);
});

test("a selected row or a row a search found shows past the fold", async () => {
  const { panel, view, press } = await mounted(longReplies);
  const last = sentry().at(-1);
  panel.selected = last.id;
  assert.ok(drawnRow(view(), last));
  panel.selected = null;

  press(view(), "report-search");
  panel.query.set_value(last.key);
  assert.ok(drawnRow(view(), last));
  // The Picks view lists every pick to agree.
  press(view(), "report-search");
  panel.view = "picks";
  assert.ok(drawnRow(view(), last));
});

test("a row with a comment thread shows past the fold", async () => {
  const commented = structuredClone(long);
  const rows = commented.report.sections.find((/** @type {any} */ section) => section.id === "sentry").rows;
  rows.at(-1).comments = [{ id: 9, author: "max", created: "2026-11-20T08:00:00Z", text: "Why this one?", resolved: false, replies: [] }];
  const { view, text } = await mounted({ ...longReplies, report: (/** @type {string[]} */ args) => (args[1] === "list" ? { revision: 6, reports: [] } : commented) });
  assert.ok(drawnRow(view(), sentry().at(-1)));
  assert.match(text(view()), /Why this one\?/);
});
