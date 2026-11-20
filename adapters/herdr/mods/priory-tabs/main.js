import { View } from "gpui-kit";
import { h_flex, Button } from "gpui-base";
import { appHome, exec, focused, layout, notify, panes, readTextFile, run, settings, shared, theme as herdrTheme, window } from "herdr";
import { priory } from "./priory.js";

const theme = /** @type {() => Record<string, import("gpui-kit").Color>} */ (herdrTheme);

// Desk and Bonjour open as center tabs; Asks and Queue are children of the layout's side panel.
const TABS = [["desk", "Desk", "priory-desk"], ["bonjour", "Bonjour", "priory-bonjour"]];
const PANELS = [["asks", "Asks"], ["queue", "Queue"]];

/** @param {any} node @param {string} id @returns {any} */
function find(node, id) {
  if (node?.id === id) return node;
  for (const child of node?.children ?? []) {
    const found = find(child, id);
    if (found) return found;
  }
  return null;
}

/** @param {string} name @param {string} mod */
function openTab(name, mod) {
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
  return priory({ exec, binary: config.binary, on: config.machine ?? null, factory: config.factory ?? null });
}

export default class PrioryTabs extends View {
  init() {
    this.client = connect();
    // A request left in shared state by an earlier session must not attach on start.
    this.attached = /** @type {{ at?: number } | null} */ (shared("priory.attach"))?.at ?? 0;
  }

  // Another Priory mod asks for a task's terminal with setShared("priory.attach", { task, at: Date.now() }).
  /** @param {number} task */
  async attach(task) {
    try {
      const command = await (await this.client).attach(task);
      run("terminal.open", { argv: ["sh", "-c", command], name: `Task ${task}` });
    } catch (error) {
      notify({ title: `Can't attach to task ${task}`, body: error instanceof Error ? error.message : String(error), level: "error" });
    }
  }

  /** @param {import("gpui-kit").Context} cx */
  render(cx) {
    const t = theme();
    const request = /** @type {{ task?: number, at?: number } | null} */ (shared("priory.attach"));
    if (request?.task != null && (request.at ?? 0) > this.attached) {
      this.attached = request.at;
      cx.spawn(async () => this.attach(request.task));
    }
    const root = /** @type {{ traffic_lights?: number[] }} */ (layout()), w = window();
    const sidebar = find(root, "sidebar"), panel = find(root, "panel"), body = find(root, "panel-body");
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
    const shownTab = (/** @type {string} */ mod) => (panes() ?? []).some((pane) => pane.mod === mod && pane.tab === focused()?.tab);
    const shownPanel = (/** @type {string} */ id) => Boolean(panel && !panel.hidden && body?.active === id);
    return h_flex().size_full().min_w(0).px(10).pl(lightsPadding).gap(3).items_center().bg(t.chrome).border_b(1).border_color(t.border)
      .children(TABS.map(([id, label, mod]) => button(id, label, shownTab(mod), () => openTab(label, mod))))
      .children(PANELS.filter(([id]) => (body?.children ?? []).some((/** @type {{ id: string }} */ child) => child.id === id)).map(([id, label]) =>
        button(id, label, shownPanel(id), () => {
          if (shownPanel(id)) return run("layout.hide", { id: "panel" });
          run("layout.select", { stack: "panel-body", child: id });
          run("layout.show", { id: "panel" });
        })));
  }
}
