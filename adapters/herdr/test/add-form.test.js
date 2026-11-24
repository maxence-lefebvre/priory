import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { all, context, draw, find, herdr, keydown, nodeOf, press, settle, text } from "./kit.js";

process.env.TZ = "UTC";
const { AddForm } = await import("../mods/priory-desk/add-form.js");
const { default: Desk } = await import("../mods/priory-desk/main.js");
const { default: Queue } = await import("../mods/priory-queue/main.js");

const fixture = (/** @type {string} */ name) => JSON.parse(readFileSync(new URL(`./fixtures/${name}.json`, import.meta.url), "utf8"));
const names = (/** @type {number} */ count, /** @type {string} */ name) => Array.from({ length: count }, (_, at) => `${name}-${at}`);
const choices = {
  workflow: names(8, "flow"), band: names(4, "band"), lane: names(12, "lane"),
  unpicked: { workflow: "default (flow-0)", band: "default", lane: "routed" },
};
const FIELDS = ["workflow", "band", "lane"];

/** A form over a client that records what it adds. */
function form() {
  /** @type {[string, Record<string, string>][]} */
  const sent = [];
  const add = async (/** @type {string} */ words, /** @type {Record<string, string>} */ picked) => { sent.push([words, picked]); return "cli:1"; };
  const made = new AddForm(async () => ({ add }), () => {});
  return { made, sent, tree: (offered = choices) => made.render({}, "mods/priory-desk/icons", offered) };
}

/** Opens or closes a field's menu, as gpui-base's Popover reports it. @param {any} tree @param {string} field @param {boolean} open */
const toggle = (tree, field, open) => find(tree, `add-${field}-menu`).on.on_open_change(open, context());

/** @param {any} tree */
const shown = (tree) => FIELDS.filter((field) => find(tree, `add-${field}-menu`).calls.open[0]);

/** The ids of the buttons on screen, which leaves out those of a closed menu. @param {any} value @returns {string[]} */
function buttons(value) {
  const node = nodeOf(value);
  if (!node) return [];
  const kids = node.kind === "popover" && !node.calls.open[0] ? node.kids.slice(0, 1) : node.kids;
  return [...(node.kind === "button" ? [node.id] : []), ...kids.flatMap(buttons)];
}

test("the closed form draws one dropdown per field, each on its default, and no choice", () => {
  const { tree } = form();
  assert.deepEqual(buttons(tree()), ["add-send", "add-close", "add-workflow", "add-band", "add-lane"]);
  assert.deepEqual(FIELDS.map((field) => text(find(tree(), `add-${field}`))), ["Workflow default (flow-0)", "Band default", "Lane routed"]);
  assert.deepEqual(FIELDS.map((field) => find(tree(), `add-${field}`).calls.accessibility_label[0]), ["Workflow: default (flow-0)", "Band: default", "Lane: routed"]);
});

test("a field with no choice draws no dropdown", () => {
  const { tree } = form();
  assert.deepEqual(buttons(tree({ ...choices, lane: [] })), ["add-send", "add-close", "add-workflow", "add-band"]);
});

test("a pick closes its menu and shows, and Enter in the field adds with that flag alone", async () => {
  const { made, sent, tree } = form();
  toggle(tree(), "band", true);
  assert.ok(buttons(tree()).includes("add-band-band-2"));
  press(tree(), "add-band-band-2");
  assert.deepEqual(shown(tree()), []);
  assert.equal(text(find(tree(), "add-band")), "Band band-2");

  made.input.set_value("Fix the docs");
  made.input.emit("submit");
  await settle();
  assert.deepEqual(sent, [["Fix the docs", { band: "band-2" }]]);
});

test("a menu's first row clears the pick", async () => {
  const { made, sent, tree } = form();
  press(tree(), "add-workflow-flow-3");
  toggle(tree(), "workflow", true);
  const first = buttons(find(tree(), "add-workflow-menu"))[1];
  assert.equal(text(find(tree(), first)), "default (flow-0)");
  press(tree(), first);
  assert.equal(text(find(tree(), "add-workflow")), "Workflow default (flow-0)");

  made.input.set_value("Fix the docs");
  made.input.emit("submit");
  await settle();
  assert.deepEqual(sent, [["Fix the docs", {}]]);
});

test("one menu shows at a time, and a late close of the first leaves the second open", () => {
  const { tree } = form();
  toggle(tree(), "workflow", true);
  toggle(tree(), "lane", true);
  toggle(tree(), "workflow", false);
  assert.deepEqual(shown(tree()), ["lane"]);
});

test("closing the form closes its menu", () => {
  const { made, tree } = form();
  toggle(tree(), "lane", true);
  made.hide(context());
  made.show(context());
  assert.deepEqual(shown(tree()), []);
});

const PANELS = /** @type {const} */ ([["priory-desk", Desk, "desk-queue"], ["priory-queue", Queue, "queue"]]);

for (const [mod, Panel, queue] of PANELS) {
  /** The panel with its Add form open and the focus off the field. */
  const opened = async () => {
    herdr.reset();
    herdr.replies = { queue: { ...fixture(queue), bands: ["now", "today"], default_workflow: "implement" }, desk: fixture("desk"), asks: fixture("desk-asks"), tasks: fixture("desk-tasks"), links: { links: [] } };
    const panel = new Panel();
    const cx = context();
    panel.init({}, cx);
    await settle();
    panel.adding.show(cx);
    return { panel, cx };
  };

  test(`${mod}: Esc closes an open menu first, then the form`, async () => {
    const { panel, cx } = await opened();
    toggle(draw(panel, cx), "workflow", true);
    keydown(draw(panel, cx), "add-workflow", "escape");
    assert.deepEqual([shown(draw(panel, cx)), panel.adding.open], [[], true]);
    keydown(draw(panel, cx), "add-workflow", "escape");
    assert.equal(panel.adding.open, false);
  });

  test(`${mod}: a key pressed on a dropdown reaches none of the panel's shortcuts`, async () => {
    const { panel, cx } = await opened();
    const tree = draw(panel, cx);
    const before = panel.selected;
    const calls = herdr.argv.length;
    for (const key of ["j", "down", "enter", "s", "r"]) keydown(tree, "add-band", key);
    assert.equal(panel.selected, before);
    assert.deepEqual([herdr.argv.length, herdr.runs], [calls, []]);
  });

  test(`${mod}: a key typed in the field goes on to the field`, async () => {
    const { panel, cx } = await opened();
    panel.adding.input.emit("focus");
    const keys = context();
    assert.ok(find(draw(panel, cx), "add-form").calls.track_focus);
    find(draw(panel, cx), "add-form").on.on_key_down({ key: "j", keystroke: "j", modifiers: {} }, keys);
    assert.equal(keys.stopped, false);
  });

  test(`${mod}: the form names the factory's default workflow and draws icons the mod has`, async () => {
    const { panel, cx } = await opened();
    const drawn = find(draw(panel, cx), "add-form");
    assert.match(text(drawn), /Workflow default \(implement\)/);
    for (const { path } of all(drawn, "svg")) assert.ok(existsSync(new URL(`../${path}`, import.meta.url)), `${path} is missing`);
  });
}
