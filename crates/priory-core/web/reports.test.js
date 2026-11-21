import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dossier, refusal } from "./reports.js";

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
