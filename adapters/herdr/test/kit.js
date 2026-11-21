// A stand-in for Herdr's `gpui-kit`, `gpui-base` and `herdr` modules, so a test can load a mod's
// main.js in Node, draw it to a tree, and press its elements. Importing this file maps the three
// module names onto it.
import { registerHooks } from "node:module";

const SELF = import.meta.url;
registerHooks({
  resolve: (specifier, context, next) => (["gpui-kit", "gpui-base", "herdr"].includes(specifier) ? { url: SELF, shortCircuit: true } : next(specifier, context)),
});

const NODE = Symbol("node");

/**
 * An element: every builder call returns it, and it keeps its id, children and event handlers.
 * @param {string} kind @param {string | null} [id] @param {Record<string, unknown>} [props]
 * @returns {any}
 */
function element(kind, id = null, props = {}) {
  const node = { kind, id, kids: /** @type {any[]} */ ([]), on: /** @type {Record<string, Function>} */ ({}), calls: /** @type {Record<string, any[]>} */ ({}), ...props };
  const self = new Proxy(node, {
    get(_target, prop) {
      if (prop === NODE) return node;
      if (typeof prop !== "string" || prop === "then") return undefined;
      return (/** @type {any[]} */ ...args) => {
        if (prop === "when") return args[0] ? args[1](self) : self;
        if (prop === "id") node.id = args[0];
        else if (prop === "child" || prop === "trigger" || prop === "content") node.kids.push(args[0]);
        else if (prop === "children") node.kids.push(...args[0]);
        else if (prop.startsWith("on_")) node.on[prop] = args[args.length - 1];
        else node.calls[prop] = args;
        return self;
      };
    },
  });
  return self;
}

/** An element's node, or the node itself, as `find` returns it. @param {any} value */
const nodeOf = (value) => (value && (value[NODE] ?? (Array.isArray(value.kids) ? value : null))) || null;

/** The path of nodes from `root` to the element with `id`, or null. @param {any} root @param {string} id */
function pathTo(root, id) {
  const node = nodeOf(root);
  if (!node) return null;
  if (node.id === id) return [node];
  for (const kid of node.kids) {
    const path = pathTo(kid, id);
    if (path) return [node, ...path];
  }
  return null;
}

/** The element with `id` in a drawn tree, or null. @param {any} root @param {string} id */
export const find = (root, id) => pathTo(root, id)?.at(-1) ?? null;

/** Every text a node and its children show, joined by spaces. @param {any} value @returns {string} */
export function text(value) {
  if (typeof value === "string") return value;
  const node = nodeOf(value);
  if (!node) return "";
  return [node.html ?? "", ...node.kids.map(text)].filter(Boolean).join(" ");
}

/** A context whose spawned work the test awaits with `settle`. */
export function context() {
  const cx = {
    stopped: false,
    rendering: false,
    notify() {
      if (cx.rendering) throw new Error("cx.notify() during render: Herdr throws, since notifying a view while it renders is a loop");
    },
    stop_propagation() { cx.stopped = true; },
    /** @param {(cx: any) => unknown} work */
    spawn(work) { Promise.resolve().then(() => work(cx)).catch(() => {}); },
    timer: { after: () => ({ cancel() {} }) },
    sleep: () => new Promise(() => {}),
    focus_handle: () => ({ focus() {} }),
    theme: () => ({ appearance: "light" }),
  };
  return cx;
}

/** Draws a panel as Herdr does, where `cx.notify()` throws. @param {{ render(cx: any): any }} panel @param {any} cx */
export function draw(panel, cx) {
  cx.rendering = true;
  try {
    return panel.render(cx);
  } finally {
    cx.rendering = false;
  }
}

/** Waits until every piece of spawned work that can finish has finished. */
export async function settle() {
  for (let round = 0; round < 20; round += 1) await new Promise((resolve) => setImmediate(resolve));
}

/**
 * Presses the element with `id` as gpui does: the mouse down bubbles from it to the root until a
 * handler stops it, and each element the down reached takes the click.
 * @param {any} root @param {string} id
 */
export function press(root, id) {
  const path = pathTo(root, id);
  if (!path) throw new Error(`no element ${id}`);
  const down = context();
  const reached = [];
  for (const node of path.reverse()) {
    node.on.on_mouse_down?.({}, down);
    reached.push(node);
    if (down.stopped) break;
  }
  for (const node of reached) if (!node.calls.disabled?.[0]) node.on.on_click?.({}, context());
}

// gpui-kit
export class View {}
export const div = () => element("div");
export const svg = (/** @type {string} */ path) => element("svg", null, { path });
export const Background = {};
export const PathBuilder = {};

// gpui-base
export const h_flex = () => element("h_flex");
export const v_flex = () => element("v_flex");
export const Button = { new: (/** @type {string} */ id) => element("button", id) };
export const TextView = { html: (/** @type {string} */ id, /** @type {string} */ html) => element("text", id, { html }) };
export const Input = { new: () => element("input") };
export const Textarea = { new: () => element("textarea") };
export const Popover = { new: (/** @type {string} */ id) => element("popover", id) };
export const ScrollHandle = { new: () => ({ scroll_to_item() {} }) };
const fieldState = () => {
  let value = "";
  return { value: () => value, set_value: (/** @type {string} */ next) => { value = next; }, on() {}, focus() {} };
};
export const InputState = { new: fieldState };
export const TextareaState = { new: fieldState };

// herdr: what the test sets and reads.
export const herdr = {
  /** @type {Record<string, unknown>} each priory verb's JSON reply */
  replies: {},
  /** @type {Map<string, unknown>} */
  shared: new Map(),
  /** @type {[string, unknown][]} */
  runs: [],
  /** @type {string[][]} */
  argv: [],
  /** @type {unknown} the layout Herdr shows */
  layout: {},
  /** @type {Record<string, string>} the files readTextFile reads, by path */
  files: {},
  reset() {
    this.replies = {};
    this.shared.clear();
    this.runs = [];
    this.argv = [];
    this.layout = {};
    this.files = {};
  },
};

/** Runs priory against `herdr.replies`; `wait` never returns, so a panel reads once. @param {string} _binary @param {string[]} args */
export async function exec(_binary, args) {
  herdr.argv.push(args);
  if (args[0] === "wait") return new Promise(() => {});
  const reply = herdr.replies[args[0]];
  if (reply === undefined) return { code: 1, stdout: "", stderr: `priory ${args[0]}: no such thing` };
  return { code: 0, stdout: JSON.stringify(reply), stderr: "" };
}
export const appHome = () => "/herdr";
export const home = () => "/home/me";
export const readTextFile = async (/** @type {string} */ path) => herdr.files[path] ?? null;
export const checking = () => false;
export const nextCommand = () => new Promise(() => {});
export const settings = () => ({ zoom: 1 });
export const run = (/** @type {string} */ command, /** @type {unknown} */ args) => { herdr.runs.push([command, args]); };
export const setShared = (/** @type {string} */ key, /** @type {unknown} */ value) => { herdr.shared.set(key, value); };
export const shared = (/** @type {string} */ key) => herdr.shared.get(key) ?? null;
export const focused = () => ({ space: "space-1", tab: "tab-1" });
export const panes = () => [];
export const spaces = () => [{ id: "space-1", active: true, machine: null, folder: "/home/me" }];
export const layout = () => herdr.layout;
export const notify = () => {};
export const theme = () => ({});
export const window = () => null;

// The Desk reads Herdr's appearance from the engine's global `window`.
Object.assign(globalThis, { window: { appearance: () => "light" } });
