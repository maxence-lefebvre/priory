import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { timeline, ungatedPairs } from "../mods/priory-flows/view.js";
import { labelSpots, pathCalls } from "../mods/priory-flows/path.js";

// Captured from `priory flows DIR --json`: flows-groom.json on examples/groom, flows.json on
// docs/reference/examples/factory.
const fixture = (name) => JSON.parse(readFileSync(new URL(`./fixtures/${name}.json`, import.meta.url), "utf8"));

test("the view model is the web view's flows.js, copied", () => {
  const source = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
  assert.equal(source("../mods/priory-flows/view.js"), source("../../../crates/priory-core/web/flows.js"),
    "cp crates/priory-core/web/flows.js adapters/herdr/mods/priory-flows/view.js");
});

test("groom lays out each step one row below the latest step that leads to it, done and stop last", () => {
  const flows = fixture("flows-groom");
  const [groom] = flows.workflows;
  const drawing = timeline(groom, ungatedPairs(groom.name, flows.paths));
  const rows = Map.groupBy(drawing.nodes, (node) => node.y);
  assert.deepEqual([...rows.values()].map((nodes) => nodes.map((node) => node.id)), [
    ["groom"], ["approve-plan", "implement-green"], ["implement"], ["review"], ["describe"], ["open-pr"], ["done", "stop"],
  ]);
  const openPr = drawing.nodes.find((node) => node.id === "open-pr");
  assert.equal(openPr.gate, "autonomous: it only opens a pull request, which a person reviews before anyone merges it");
  assert.deepEqual(drawing.nodes.filter((node) => node.terminal).map((node) => node.id), ["done", "stop"]);
  // review's fix loops back to implement, so it runs down a lane.
  assert.deepEqual(drawing.edges.filter((edge) => edge.back).map((edge) => `${edge.from}>${edge.to}`), ["review>implement"]);
});

test("each edge on a path to open-pr with no person's decision is ungated, and a person's answer never is", () => {
  const flows = fixture("flows-groom");
  const [groom] = flows.workflows;
  const edges = timeline(groom, ungatedPairs(groom.name, flows.paths)).edges;
  assert.deepEqual(edges.filter((edge) => edge.ungated).map((edge) => `${edge.from}>${edge.to}`), [
    "groom>implement-green", "groom>implement", "implement>review", "review>describe", "describe>open-pr", "implement-green>open-pr",
  ]);
  assert.equal(edges.find((edge) => edge.from === "approve-plan" && edge.to === "implement").ungated, false);
});

test("a factory with several workflows draws each, its steps then done and stop", () => {
  const flows = fixture("flows");
  assert.deepEqual(flows.workflows.map((workflow) => [workflow.name, timeline(workflow, ungatedPairs(workflow.name, flows.paths)).nodes.length]), [
    ["implement", 7], ["poll", 3], ["sweep", 5],
  ]);
});

test("an SVG path becomes PathBuilder calls, scaled, with H and V as lines from the current point", () => {
  assert.deepEqual(pathCalls("M26 0H124A26 26 0 0 1 124 52Z", 0.5), [
    ["move_to", 13, 0], ["line_to", 62, 0], ["arc_to", 13, 13, 0, false, true, 62, 26], ["close"],
  ]);
  // SVG gives a curve's controls before its end; PathBuilder takes the end first.
  assert.deepEqual(pathCalls("M75 64C75 94 30 106 30 136V180L0 1.5", 1), [
    ["move_to", 75, 64], ["cubic_bezier_to", 30, 136, 75, 94, 30, 106], ["line_to", 30, 180], ["line_to", 0, 1.5],
  ]);
});

test("an upright lane label that would cover another moves down a line, and the rest stay at their edge's middle", () => {
  const flows = fixture("flows-groom");
  const [groom] = flows.workflows;
  const edges = timeline(groom, ungatedPairs(groom.name, flows.paths)).edges;
  const spots = labelSpots(edges, 1);
  const at = (from, to) => spots[edges.findIndex((edge) => edge.from === from && edge.to === to)];
  // rejected and failed share their lanes' middle, 42 px apart: the later one moves 16 px down.
  assert.deepEqual([at("approve-plan", "stop"), at("implement-green", "stop")], [{ x: 365, y: 422 }, { x: 407, y: 438 }]);
  assert.deepEqual(at("groom", "implement"), { x: 351, y: 142 });
  assert.deepEqual(at("groom", "approve-plan"), { x: 135.5, y: 86 });
});
