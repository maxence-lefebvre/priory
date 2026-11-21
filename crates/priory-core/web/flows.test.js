import { test } from "node:test";
import assert from "node:assert/strict";
import { KINDS, edgeStyle, lockBadge, nodeShape, timeline, ungatedPairs } from "./flows.js";

// `step` and `workflow` as `GET /api/flows` sends them.
const edge = (outcome, to, by) => ({ outcome, to, by });
const implement = {
  name: "implement",
  start: "plan",
  steps: [
    { id: "plan", decides: "model", effect: "reversible", kind: "reversible", edges: [edge("green", "implement", "model"), edge("red", "approve-plan", "model")] },
    { id: "approve-plan", decides: "human", effect: "none", kind: "ask", options: ["approved", "rejected"], edges: [edge("approved", "implement", "human"), edge("rejected", "stop", "human")] },
    { id: "implement", decides: "model", effect: "reversible", kind: "reversible", edges: [edge("failed", "stop", "model"), edge("passed", "open-pr", "model")] },
    { id: "open-pr", decides: "code", effect: "irreversible", kind: "irreversible", autonomous: "it only labels", edges: [edge("failed", "stop", "code"), edge("opened", "done", "code")] },
  ],
};

test("each step kind has its own silhouette", () => {
  assert.deepEqual(KINDS, ["read", "reversible", "irreversible", "ask", "answer", "verdict"]);
  const shapes = KINDS.map((kind) => nodeShape(kind, 150, 52).d);
  assert.equal(new Set(shapes).size, KINDS.length);
  assert.equal(nodeShape("nonsense", 150, 52).d, nodeShape("reversible", 150, 52).d);
});

test("each decider has its own line style, in dashes and weight as well as color", () => {
  const styles = ["code", "model", "human", "asleep"].map((by) => edgeStyle(by));
  assert.equal(new Set(styles.map((style) => `${style.dash}|${style.width}`)).size, 4);
  assert.equal(edgeStyle("human").dash, "");
  assert.ok(edgeStyle("human").width > edgeStyle("code").width);
  assert.equal(edgeStyle("unknown").dash, edgeStyle("code").dash);
});

test("a step pair on a path with no human decision is ungated, and only that pair", () => {
  const paths = [
    { workflow: "implement", path: ["plan", "implement", "open-pr"] },
    { workflow: "other", path: ["a", "b"] },
  ];
  assert.deepEqual([...ungatedPairs("implement", paths)].sort(), ["implement>open-pr", "plan>implement"]);
  assert.deepEqual([...ungatedPairs("none", paths)], []);
});

test("the timeline puts a step below every step that leads to it and ends in done and stop", () => {
  const drawing = timeline(implement, new Set(["plan>implement", "implement>open-pr"]));
  const row = (id) => drawing.nodes.find((node) => node.id === id).y;
  assert.ok(row("plan") < row("approve-plan"));
  assert.ok(row("approve-plan") < row("implement"));
  assert.ok(row("implement") < row("open-pr"));
  assert.ok(row("open-pr") < row("stop"));
  assert.equal(drawing.nodes.find((node) => node.id === "done").terminal, true);
  assert.equal(drawing.edges.length, 8);
});

test("the ungated edges are marked, and an edge a human decides never is", () => {
  const drawing = timeline(implement, new Set(["plan>implement", "approve-plan>implement"]));
  const marked = drawing.edges.filter((edge) => edge.ungated).map((edge) => `${edge.from}>${edge.to}`);
  assert.deepEqual(marked, ["plan>implement"]);
});

test("an irreversible step carries its gate, and a loop back is drawn", () => {
  const looping = structuredClone(implement);
  looping.steps[1].edges[1] = edge("rejected", "plan", "human");
  const drawing = timeline(looping, new Set());
  assert.equal(drawing.nodes.find((node) => node.id === "open-pr").gate, "autonomous: it only labels");
  assert.equal(drawing.nodes.find((node) => node.id === "plan").gate, null);
  const gated = structuredClone(implement);
  delete gated.steps[3].autonomous;
  gated.steps[3].approved_by = ["approve-pr"];
  assert.equal(timeline(gated, new Set()).nodes.find((node) => node.id === "open-pr").gate, "approved by approve-pr");
  gated.steps[3].approved_by = [];
  assert.equal(timeline(gated, new Set()).nodes.find((node) => node.id === "open-pr").gate, "no approval");
  assert.equal(drawing.edges.find((edge) => edge.from === "approve-plan" && edge.to === "plan").back, true);
});

test("the timeline is at most two nodes wide for a phone, and every node has a text alternative", () => {
  const drawing = timeline(implement, new Set());
  const perRow = Object.values(Object.groupBy(drawing.nodes, (node) => node.y)).map((row) => row.length);
  assert.ok(Math.max(...perRow) <= 2);
  for (const node of drawing.nodes) assert.match(node.label, /\w/);
  assert.match(drawing.text, /plan/);
  assert.match(drawing.text, /open-pr: irreversible write, failed to stop/);
  assert.doesNotMatch(drawing.text, /write write/);
});

const fan = (count) => ({
  name: "fan",
  start: "start",
  steps: [
    { id: "start", decides: "model", effect: "reversible", kind: "reversible", edges: Array.from({ length: count }, (_, at) => edge(`o${at}`, `s${at}`, "model")) },
    ...Array.from({ length: count }, (_, at) => ({ id: `s${at}`, decides: "code", effect: "read", kind: "read", edges: [edge("ok", "done", "code")] })),
  ],
});

test("a row holds at most two steps, so a three way branch stays readable on a phone", () => {
  const drawing = timeline(fan(5), new Set());
  const perRow = Object.values(Object.groupBy(drawing.nodes, (node) => node.y)).map((row) => row.length);
  assert.ok(Math.max(...perRow) <= 2);
  assert.equal(drawing.nodes.filter((node) => /^s\d/.test(node.id)).length, 5);
});

test("outcomes that share both ends take their own lines and labels", () => {
  const poll = { name: "poll", start: "poll", steps: [{ id: "poll", decides: "code", effect: "read", kind: "read", edges: [edge("failed", "stop", "code"), edge("found", "done", "code"), edge("ok", "stop", "code")] }] };
  const [failed, , ok] = timeline(poll, new Set()).edges;
  assert.notEqual(failed.d, ok.d);
  assert.notDeepEqual(failed.mid, ok.mid);
});

test("an irreversible step's padlock has a size in the drawing's own units", () => {
  assert.deepEqual(lockBadge(150), { x: 110, y: 6, size: 15 });
});
