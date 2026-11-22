import { View } from "gpui-kit";
import { h_flex, Button } from "gpui-base";
import { createHash } from "crypto";
import { unlink } from "fs/promises";
import { appHome, checking, exec, focused, home as herdrHome, layout, layouts as layoutNames, nextCommand, notify, panes, readTextFile, run, settings, shared, spaces, theme as herdrTheme, window, writeTextFile } from "herdr";
import { OWN_ID, buttons, layouts, reveal } from "./layout.js";
import { priory } from "./priory.js";

const theme = /** @type {() => Record<string, import("gpui-kit").Color>} */ (herdrTheme);

// The bar's label for each panel.
const LABELS = { desk: "Desk", asks: "Asks", queue: "Queue", tasks: "Tasks", runs: "Runs", inbox: "Inbox", reports: "Bonjour", workflow: "Flows", costs: "Costs" };

/** @param {any} node @param {string} id @returns {any} */
function find(node, id) {
  if (node?.id === id) return node;
  for (const child of node?.children ?? []) {
    const found = find(child, id);
    if (found) return found;
  }
  return null;
}

// A tab shows in `main`, which a view can hold in a stack behind another panel.
const showMain = () => { for (const [command, args] of reveal(layout(), "main")) run(command, args); };

/** @param {string} name @param {string} mod */
function openTab(name, mod) {
  showMain();
  const open = (panes() ?? []).find((pane) => pane.mod === mod);
  if (open) return run("tab.select", { id: open.tab });
  run("tab.new", { space: focused()?.space, name, mod });
}

// settings.json's "priory" object: { binary, machine, factory }, each optional.
async function connect() {
  /** @type {{ binary?: string, machine?: string, factory?: string }} */
  let config = {};
  try {
    config = JSON.parse(await readTextFile(`${appHome()}/settings.json`) ?? "{}").priory ?? {};
  } catch {
    config = {};
  }
  const on = config.machine ?? null;
  let binary = config.binary ?? "~/.local/bin/priory-oss";
  // exec expands no `~`, and only the machine that runs priory knows its home.
  const found = on ? await exec("printenv", ["HOME"], { on }) : { code: 0, stdout: herdrHome() };
  const home = found.code === 0 ? found.stdout.trim() : "";
  if (!home.startsWith("/")) throw new Error(`No home folder on ${on ?? "this machine"}`);
  if (binary.startsWith("~/")) binary = home + binary.slice(1);
  return { machine: on, home, client: priory({ exec, binary, on, factory: config.factory ?? null }) };
}

/** The 12 hex digits `adapters/herdr/layout` puts in a layout's root id. @param {string} views what `priory views --json` printed */
const viewsHash = (views) => /** @type {string} */ (createHash("sha256").update(views.trim()).digest("hex")).slice(0, 12);

/** The root id of a layout file's text when it is a string, or null. @param {string | null} text */
function rootId(text) {
  try {
    const id = text == null ? null : JSON.parse(text).id;
    return typeof id === "string" ? id : null;
  } catch {
    return null;
  }
}

export default class PrioryTabs extends View {
  /** @param {import("gpui-kit").Context} cx */
  init(_props, cx) {
    /** @type {ReturnType<typeof connect> | null} */
    this.priory = null;
    // A request left in shared state by an earlier session must not attach, or open a task, on start.
    this.attached = /** @type {{ at?: number } | null} */ (shared("priory.attach"))?.at ?? 0;
    this.opened = /** @type {{ at?: number } | null} */ (shared("priory.task.open"))?.at ?? 0;
    // herdr-app check runs init for real: no priory call.
    if (checking()) return;
    // The change notice's action, `priory-tabs.regenerate`, arrives here.
    cx.spawn(async () => {
      for (;;) if ((await nextCommand()) === "regenerate") await this.regenerate();
    });
    cx.spawn(async () => this.offerToRegenerate());
  }

  connect() {
    // Connects on first use, and again after a failure, so a machine that comes back works.
    return (this.priory ??= connect().catch((error) => { this.priory = null; throw error; }));
  }

  // Asks once, at start: a person changes views rarely, and the notice stays until clicked or closed.
  async offerToRegenerate() {
    const id = /** @type {{ id?: unknown } | null} */ (layout())?.id;
    if (typeof id !== "string" || !OWN_ID.test(id)) return;
    let views;
    try {
      views = await (await this.connect()).client.views();
    } catch {
      return;
    }
    if (id === `priory-generated-${viewsHash(views)}`) return;
    notify({ title: "The view files changed", body: "Click to regenerate the Priory layouts from them.", level: "info", action: { name: "priory-tabs.regenerate" } });
  }

  // The same steps as `install` for the layouts, in the layouts folder of this Herdr. The installed priory.json is the frame.
  async regenerate() {
    try {
      const folder = `${appHome()}/layouts`;
      const views = await (await this.connect()).client.views();
      const frame = await readTextFile(`${folder}/priory.json`);
      if (!OWN_ID.test(rootId(frame) ?? "")) throw new Error("layouts/priory.json is not a layout this adapter wrote. Move it away, then run install again.");
      const files = layouts(JSON.parse(views).views, JSON.parse(/** @type {string} */ (frame)), viewsHash(views));
      /** @type {Record<string, string | null>} */
      const old = {};
      for (const file of Object.keys(files)) {
        old[file] = await readTextFile(`${folder}/${file}`);
        if (old[file] != null && !OWN_ID.test(rootId(old[file]) ?? "")) throw new Error(`layouts/${file} is not a layout this adapter wrote. Move it away, then regenerate again.`);
      }
      // priory.json last: a failed write leaves its old hash, so the bar offers again at the next start.
      const order = Object.keys(files).sort((a, b) => Number(a === "priory.json") - Number(b === "priory.json"));
      for (const file of order) await writeTextFile(`${folder}/${file}`, `${JSON.stringify(files[file], null, 2)}\n`, old[file]);
      for (const name of layoutNames()) {
        const file = `${name}.json`;
        if (!name.startsWith("priory-") || Object.hasOwn(files, file)) continue;
        if (OWN_ID.test(rootId(await readTextFile(`${folder}/${file}`)) ?? "")) await unlink(`${folder}/${file}`);
      }
      notify({ title: "Regenerated the Priory layouts", level: "success" });
    } catch (error) {
      notify({ title: "Can't regenerate the Priory layouts", body: error instanceof Error ? error.message : String(error), level: "error" });
    }
  }

  // Another Priory mod asks for a task's terminal with setShared("priory.attach", { task, at: Date.now(), readOnly }).
  /** @param {number} task @param {import("herdr").SpaceInfo | undefined} space the space shown when asked @param {boolean} readOnly */
  async attach(task, space, readOnly) {
    try {
      const { machine, home, client } = await this.connect();
      if (!space || space.machine !== machine) throw new Error(`Open a space on ${machine ?? "this machine"}, where Priory runs, then attach again.`);
      const command = await client.attach(task, { readOnly });
      showMain();
      run("tab.new", { space: space.id, folder: space.folder || home, name: readOnly ? `Watch task ${task}` : `Task ${task}`, argv: ["sh", "-c", command] });
    } catch (error) {
      notify({ title: `Can't attach to task ${task}`, body: error instanceof Error ? error.message : String(error), level: "error" });
    }
  }

  /** @param {import("gpui-kit").Context} cx */
  render(cx) {
    const t = theme();
    const request = /** @type {{ task?: number, at?: number, readOnly?: boolean } | null} */ (shared("priory.attach"));
    if (request?.task != null && (request.at ?? 0) > this.attached) {
      this.attached = request.at;
      const space = (spaces() ?? []).find((candidate) => candidate.active);
      cx.spawn(async () => this.attach(request.task, space, Boolean(request.readOnly)));
    }
    // Another Priory mod opens a task with setShared("priory.task.open", { task, at: Date.now() }); the Task panel reads which.
    const opening = /** @type {{ at?: number } | null} */ (shared("priory.task.open"));
    if ((opening?.at ?? 0) > this.opened) {
      this.opened = opening?.at ?? 0;
      cx.spawn(async () => openTab("Task", "priory-task"));
    }
    const root = /** @type {{ traffic_lights?: number[] }} */ (layout()), w = window();
    const sidebar = find(root, "sidebar");
    // This bar is the top bar's first block, so it keeps clear of the macOS traffic lights.
    const lightsPadding = w?.platform === "macos" && w.titlebar === "none" && !w.fullscreen && root?.traffic_lights
      ? Math.max(10, (root.traffic_lights[0] + 64) / Number(settings().zoom || 1) - (sidebar?.hidden ? 0 : sidebar?.size?.px ?? 0))
      : 10;
    /** @param {unknown} _e @param {import("gpui-kit").Context} cx */
    const stop = (_e, cx) => cx.stop_propagation();
    /** @param {string} id @param {string} label @param {boolean} on @param {() => void} action */
    const button = (id, label, on, action) => Button.new(`priory-${id}`).selected(on).h(26).px(8).flex_none().items_center().rounded(6)
      .text_size(12).bg(on ? t.selected : t.chrome).text_color(on ? t.foreground : t.muted)
      .hover(s => s.bg(t.hover)).active(s => s.bg(t.selected)).focus(s => s.bg(t.selected))
      .on_mouse_down("left", stop).on_click(action).child(label);
    // A tab is shown when no stack above it shows another and nothing above it is hidden.
    const shown = (/** @type {string} */ id) => reveal(root, id).length === 0;
    // An aside toggles. A shown tab gives the center back to main, or the terminals would stay hidden.
    /** @param {{ id: string, aside: boolean }} item */
    const press = ({ id, aside }) => {
      if (aside) return run("layout.toggle", { id });
      if (shown(id)) return run("layout.select", { stack: "center", child: "main" });
      for (const [command, args] of reveal(root, id)) run(command, args);
    };
    const items = buttons(root);
    // An installed layout from before the asides has none of their blocks, until `install` runs again.
    if (!items.length) return h_flex().id("priory-bar").size_full().px(10).pl(lightsPadding).items_center().bg(t.chrome).border_b(1).border_color(t.border).text_size(12).text_color(t.muted).child("The installed layout is older than this bar: run install again");
    // The panels can hold more buttons than the bar's width.
    return h_flex().id("priory-bar").overflow_x_scroll().size_full().min_w(0).px(10).pl(lightsPadding).gap(3).items_center().bg(t.chrome).border_b(1).border_color(t.border)
      .children(items.map((item) => button(item.id, LABELS[/** @type {keyof typeof LABELS} */ (item.panel)], shown(item.id), () => press(item))));
  }
}
