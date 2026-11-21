import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { around, dossier, refusal } from "./reports.js";

process.env.TZ = "UTC";

// `/api/reports/latest` as the Herdr Bonjour tests capture it from `priory report show --json`.
const fixture = (name) => JSON.parse(readFileSync(new URL(`../../../adapters/herdr/test/fixtures/${name}`, import.meta.url), "utf8"));
const read = fixture("report.json");
const now = Date.parse("2026-11-20T08:30:00Z");
const rows = (view) => view.sections.flatMap((section) => section.rows);

test("the web view and the Bonjour panel share one report view model", () => {
  const source = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
  // Each imports autolink from its own copy of the links module.
  const web = source("./reports.js").replace('from "./links.js"', 'from "./priory.js"');
  assert.equal(web, source("../../../adapters/herdr/mods/priory-bonjour/view.js"));
});

test("sections keep the report's own order", () => {
  assert.deepEqual(dossier(read, { now }).sections.map((section) => section.id), ["reading", "slack", "sentry", "agree", "blocked", "moved"]);
});

test("a pick is open with its options, agreed once chosen, and applied once the workflow says so", () => {
  const picks = rows(dossier(read, { now })).filter((row) => row.pick).map((row) => [row.id, row.pick.state, row.pick.chosen]);
  assert.deepEqual(picks, [
    ['["sentry","fnd-2409"]', "applied", "Take"],
    ['["sentry","fnd-2413"]', "open", null],
    ['["agree",0]', "agreed", "Take"],
    ['["agree",1]', "open", null],
  ]);
});

test("an agreed pick keeps its options to change, and an applied one offers none", () => {
  const picks = Object.fromEntries(rows(dossier(read, { now })).filter((row) => row.pick).map((row) => [row.id, row.pick]));
  assert.deepEqual(picks['["agree",0]'].options.map((option) => [option.label, option.key]), [["Take", "1"], ["Later", "2"], ["Drop", "3"]]);
  assert.deepEqual(picks['["sentry","fnd-2409"]'].options, []);
  // A change refused because someone else changed it, or the workflow applied it meanwhile.
  const sent = { pick: "2026-11-20_agree_priory-schedules", option: "Later", revision: 3, from: "Take" };
  const changed = structuredClone(read);
  changed.report.sections[3].rows[0].pick.chosen = "Drop";
  assert.equal(refusal(sent, changed, "pick is at revision 4"), "Already picked Drop.");
  const applied = structuredClone(read);
  applied.report.sections[3].rows[0].pick.applied = "scheduled";
  assert.equal(refusal(sent, applied, "pick is already Take"), "Already picked Take.");
  const moved = { revision: 8, report: { ...read.report, revision: 5 } };
  assert.equal(refusal(sent, moved, "pick is at revision 5"), "The report changed to revision 5. Read it and pick again.");
});

test("a search keeps the rows whose key, text, meta, owner or threads hold it, in their sections", () => {
  const found = (query, report = read) => dossier(report, { now, query }).sections.map((section) => [section.id, section.rows.map((row) => row.key ?? row.id)]);
  assert.deepEqual(found("pri-6"), [["blocked", ["PRI-69"]], ["moved", ["PRI-63", "PRI-60"]]]);
  assert.deepEqual(found("  on max "), [["blocked", ["PRI-55"]]]);
  assert.deepEqual(found("18:00"), [["sentry", ["FND-2409"]]]);
  assert.equal(dossier(read, { now, query: "pri-6" }).found, 3);
  assert.equal(dossier(read, { now }).found, null);
  assert.deepEqual(found("nothing like this"), []);
  const commented = structuredClone(read);
  commented.report.sections[2].rows[1].comments = [{ id: 1, author: "person", text: "Check the release branch", created: "2026-11-20T08:05:00Z", resolved: false,
    replies: [{ id: 2, author: "agent", text: "It is clean", created: "2026-11-20T08:07:00Z" }] }];
  assert.deepEqual(found("is clean", commented), [["sentry", ["FND-2413"]]]);
  // The section keeps its own count; the search counts what it found.
  assert.equal(dossier(read, { now, query: "pri-6" }).sections[1].count, 2);
});

test("threads sit under their row, oldest first, with their replies and resolved state", () => {
  const report = structuredClone(read);
  report.report.sections[2].rows[1].comments = [
    { id: 1, author: "person", text: "Check the release branch first", created: "2026-11-20T08:05:00Z", resolved: false,
      replies: [{ id: 2, author: "agent", text: "It is clean", created: "2026-11-20T08:07:00Z" }] },
    { id: 3, author: "person", text: "Done", created: "2026-11-20T08:10:00Z", resolved: true, replies: [] },
  ];
  const row = rows(dossier(report, { now })).find((candidate) => candidate.target === "fnd-2413");
  assert.deepEqual(row.threads, [
    { id: 1, author: "person", when: "08:05", text: "Check the release branch first", resolved: false,
      replies: [{ id: 2, author: "agent", when: "08:07", text: "It is clean" }] },
    { id: 3, author: "person", when: "08:10", text: "Done", resolved: true, replies: [] },
  ]);
});

test("a refused pick says whether another choice took it or the report moved on", () => {
  const sent = { pick: "2026-11-20_sentry_fnd-2413", option: "Take", revision: 3 };
  const revised = fixture("report-revised.json");
  assert.equal(refusal(sent, revised, "pick 2026-11-20_sentry_fnd-2413 is already Later"), "Already picked Later.");
  assert.equal(refusal(sent, null, "the connection dropped"), "Not sent: the connection dropped.");
});

test("older and newer go to the reports written before and after the one shown", () => {
  // `priory report list --json`, newest first.
  const list = [{ task: 9, date: "2026-11-21" }, { task: 4, date: "2026-11-20" }, { task: 7, date: "2026-11-19" }];
  assert.deepEqual(around(list, 9), { older: 4, newer: null });
  assert.deepEqual(around(list, 4), { older: 7, newer: 9 });
  assert.deepEqual(around(list, 7), { older: null, newer: 4 });
  // A report the list does not hold yet, such as one written after the list was read.
  assert.deepEqual(around(list, 12), { older: null, newer: null });
});
