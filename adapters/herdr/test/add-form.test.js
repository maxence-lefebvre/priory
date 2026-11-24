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
  return [...(node.kind === "button" || node.calls.role?.[0] === "list_box_option" ? [node.id] : []), ...kids.flatMap(buttons)];
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

test("a picked value the choices drop stays shown, sent and clearable", async () => {
  const { made, sent, tree } = form();
  press(tree(), "add-lane-lane-3");
  const without = { ...choices, lane: [] };
  assert.equal(text(find(tree(without), "add-lane")), "Lane lane-3");
  made.input.set_value("Fix the docs");
  made.input.emit("submit");
  await settle();
  assert.deepEqual(sent, [["Fix the docs", { lane: "lane-3" }]]);

  press(tree(), "add-lane-lane-3");
  press(tree(without), "add-clear-lane");
  assert.equal(find(tree(without), "add-lane"), null);
  made.input.set_value("Fix the tests");
  made.input.emit("submit");
  await settle();
  assert.deepEqual(sent[1], ["Fix the tests", {}]);
});

test("a listed pick is offered once", () => {
  const { tree } = form();
  press(tree(), "add-band-band-1");
  assert.deepEqual(buttons(find(tree(), "add-band-list")), ["add-clear-band", ...names(4, "band").map((name) => `add-band-${name}`)]);
});

/** The highlighted row's id. @param {any} tree @param {string} field */
const highlighted = (tree, field) => all(find(tree, `add-${field}-list`), "h_flex").filter((row) => row.calls.aria_active_descendant).map((row) => row.id);

test("the arrow keys move the highlight, scroll it into view, stop at both ends, and Enter picks it", () => {
  const { made, tree } = form();
  made.show(context());
  toggle(tree(), "lane", true);
  const list = find(tree(), "add-lane-list");
  assert.equal(list.calls.track_focus[0], find(tree(), "add-lane-menu").calls.track_focus[0]);
  assert.deepEqual(highlighted(tree(), "lane"), ["add-clear-lane"]);
  for (let at = 0; at < 10; at += 1) keydown(tree(), "add-lane-list", "down");
  const scroll = find(tree(), "add-lane-list").calls.track_scroll[0];
  assert.deepEqual([highlighted(tree(), "lane"), scroll.shown], [["add-lane-lane-9"], 10]);
  for (let at = 0; at < 9; at += 1) keydown(tree(), "add-lane-list", "down");
  assert.deepEqual([highlighted(tree(), "lane"), scroll.shown], [["add-lane-lane-11"], 12]);
  for (let at = 0; at < 20; at += 1) keydown(tree(), "add-lane-list", "up");
  assert.deepEqual([highlighted(tree(), "lane"), scroll.shown], [["add-clear-lane"], 0]);
  keydown(tree(), "add-lane-list", "down");
  keydown(tree(), "add-lane-list", "enter");
  assert.deepEqual([made.picked, shown(tree())], [{ lane: "lane-0" }, []]);
});

test("a menu opens on its pick, and Space on the first row clears it", () => {
  const { made, tree } = form();
  press(tree(), "add-lane-lane-9");
  toggle(tree(), "lane", true);
  assert.deepEqual([highlighted(tree(), "lane"), find(tree(), "add-lane-list").calls.track_scroll[0].shown], [["add-lane-lane-9"], 10]);
  for (let at = 0; at < 10; at += 1) keydown(tree(), "add-lane-list", "up");
  keydown(tree(), "add-lane-list", "space");
  assert.deepEqual(made.picked, {});
});

test("a held Enter that opened the menu picks nothing", () => {
  const { made, tree } = form();
  toggle(tree(), "lane", true);
  keydown(tree(), "add-lane-list", "down");
  find(tree(), "add-lane-list").on.on_key_down({ key: "enter", is_held: true }, context());
  assert.deepEqual([made.picked, shown(tree())], [{}, ["lane"]]);
});

test("no menu row takes Tab, and a long name truncates with its full name as the label", () => {
  const { tree } = form();
  const name = "lane-".repeat(40);
  const offered = { ...choices, lane: [name] };
  press(tree(offered), `add-lane-${name}`);
  assert.deepEqual(all(find(tree(offered), "add-lane-list"), "button"), []);
  assert.equal(find(tree(offered), "add-lane").calls.accessibility_label[0], `Lane: ${name}`);
  assert.equal(find(tree(offered), `add-lane-${name}`).calls.accessibility_label[0], name);
  for (const id of ["add-lane", `add-lane-${name}`]) {
    assert.ok(all(find(tree(offered), id), "div").some((part) => part.calls.truncate && text(part) === name), id);
  }
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

  test(`${mod}: one Esc closes the form once its open menu's dropdown is gone`, async () => {
    const { panel, cx } = await opened();
    toggle(draw(panel, cx), "band", true);
    panel.adding.listFocus.focus();
    herdr.replies.queue = { .../** @type {any} */ (herdr.replies.queue), bands: [] };
    herdr.wake();
    await settle();
    assert.equal(find(draw(panel, cx), "add-band"), null);
    assert.equal(herdr.focused, panel.adding.focus);
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
