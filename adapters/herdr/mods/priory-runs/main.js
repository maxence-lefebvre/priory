import { View, div, svg } from "gpui-kit";
import { h_flex, v_flex, Button, Popover } from "gpui-base";
import { appHome, exec, home as herdrHome, readTextFile, setShared } from "herdr";
import { follow, priory } from "./priory.js";
import { FONTS, palette, STRUCTURE as S } from "./tokens.js";
import { filtered } from "./tasks.js";
import { duration, RUN_FACETS, RUN_VIEWS, runState, runViews, tokenText } from "./view.js";

const RETRY_MS = 3000;
// Recent keeps the live runs and those started in the last 7 days, as `/api/runs` does.
const WEEK_MS = 7 * 24 * 3600_000;
const ICON = (/** @type {string} */ name) => `mods/priory-runs/icons/${name}.svg`;
const LABELS = /** @type {Record<string, string>} */ ({ recent: "Recent", live: "Live" });
const EMPTY = /** @type {Record<string, string>} */ ({
  recent: "No run in the last 7 days. A run shows here once an agent starts a step.",
  live: "No run is live. Runs show here while an agent works.",
});
// A run state's tile (DESIGN.md, Run states): its fill, its ink and its icon; null draws a dot.
/** @type {Record<string, [string, string, string | null]>} */
const TILES = {
  running: ["tile-live", "on-tile-live", null], done: ["tile-ok", "on-tile", "check"], moved: ["tile-muted", "on-tile", "arrow-right"],
  asleep: ["tile-muted", "on-tile", "moon"], blocked: ["tile-warn", "on-tile", "hand"], lost: ["tile-warn", "on-tile", "unplug"],
  restarted: ["tile-muted", "on-tile", "rotate-ccw"], cancelled: ["tile-muted", "on-tile", "x"], dropped: ["tile-muted", "on-tile", "minus"],
  exited: ["tile-danger", "on-tile", "log-out"], failed: ["tile-danger", "on-tile", "circle-x"],
};
// A run state's text color; the other states take --text-2.
const ROLE = /** @type {Record<string, string>} */ ({ running: "action-text", done: "ok-text", blocked: "warn-text", lost: "warn-text", exited: "danger-text", failed: "danger-text" });

/** @typedef {Record<string, import("gpui-kit").Color>} Palette */

// settings.json's "priory" object: { binary, machine, factory, theme }, each optional. Mirrors priory-costs.
async function connect() {
  /** @type {{ binary?: string, machine?: string, factory?: string, theme?: string }} */
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
  return { theme: config.theme ?? "indigo", client: priory({ exec, binary, on, factory: config.factory ?? null }) };
}

const message = (/** @type {unknown} */ error) => (error instanceof Error ? error.message : String(error));

/** How long ago a run started: "10 min", "4 h", "3 d". @param {string} started @param {number} now */
function ago(started, now) {
  const minutes = Math.max(0, Math.floor((now - Date.parse(started)) / 60000));
  if (minutes < 60) return `${minutes} min`;
  return minutes < 1440 ? `${Math.floor(minutes / 60)} h` : `${Math.floor(minutes / 1440)} d`;
}

export default class PrioryRuns extends View {
  /** @param {unknown} _props @param {import("gpui-kit").AsyncContext} cx */
  init(_props, cx) {
    /** @type {Awaited<ReturnType<typeof connect>> | null} */
    this.priory = null;
    /** @type {any[] | null} `priory runs --json`'s runs, newest first */
    this.runs = null;
    /** @type {string | null} */
    this.error = null;
    this.view = "recent";
    /** @type {Map<string, string>} each picked facet's value */
    this.picks = new Map();
    /** @type {string | null} the facet whose menu is open */
    this.popover = null;
    this.font = FONTS.indigo;
    /** @type {ReturnType<typeof priory>} the client of the last read, which its wait uses */
    let client;
    cx.spawn((cx) => follow({
      read: async () => {
        this.priory ??= await connect();
        client = this.priory.client;
        const read = await client.runs();
        [this.runs, this.error] = [read.runs, null];
        cx.notify();
        return read.revision;
      },
      wait: (since) => client.wait(since),
      sleep: (ms) => cx.sleep(ms),
      failed: (error) => {
        this.priory = null;
        this.error = message(error);
        cx.notify();
      },
      retry: RETRY_MS,
    }));
  }

  /** @param {import("gpui-kit").Context} cx */
  render(cx) {
    const theme = this.priory?.theme ?? "indigo";
    const t = /** @type {Palette} */ (palette(theme, cx.theme().appearance));
    this.font = FONTS[/** @type {keyof typeof FONTS} */ (theme)] ?? FONTS.indigo;
    const now = Date.now();
    const recent = (this.runs ?? []).filter((run) => !run.ended || now - Date.parse(run.started) < WEEK_MS);
    const views = /** @type {Record<string, any[]>} */ (runViews(recent));
    const shown = filtered(views[this.view], this.picks, /** @type {any} */ (RUN_FACETS));
    return v_flex().id("priory-runs").size_full().min_h_0().bg(t.bg).text_color(t.text).font_family(this.font.ui).text_size(S["type-body"][0])
      .child(h_flex().flex_none().h(S["pane-head"]).px(S["space-4"]).items_center().border_b(1).border_color(t.line).child(div().font_weight(650).child("Runs")))
      .when(Boolean(this.error), (el) => el.child(div().flex_none().px(S["space-4"]).py(6).whitespace_normal().text_size(S["type-small"][0])
        .text_color(t["warn-text"]).border_b(1).border_color(t.line)
        .child(`Could not read the runs: ${this.error}. Check that priory runs, then it reads again on its own.`)))
      .child(this.filters(views, t))
      .child(v_flex().id("runs-body").flex_1().min_h_0().overflow_y_scrollbar().px(S["space-2"]).py(S["space-2"]).children(this.body(shown, t, now)));
  }

  /** @param {any[]} shown @param {Palette} t @param {number} now */
  body(shown, t, now) {
    const note = (/** @type {string} */ line) => [div().p(S["space-2"]).whitespace_normal().text_color(t["text-2"]).child(line)];
    if (!this.runs) return this.error ? [] : note("Reading the runs…");
    if (!shown.length) return note(this.picks.size ? "No run matches these filters." : EMPTY[this.view]);
    return shown.map((run) => this.row(run, t, now));
  }

  /** The saved views with their counts, then a chip per facet. @param {Record<string, any[]>} views @param {Palette} t */
  filters(views, t) {
    const chip = (/** @type {string} */ id, /** @type {boolean} */ on) => Button.new(id).h(S["pane-chip"]).px(10).flex().flex_none().items_center().gap(4)
      .selected(on).text_size(S["type-small"][0]).focus((s) => s.border(2).border_color(t["action-text"]));
    /** @param {[string, string, (run: any) => string[]]} facet */
    const picker = ([key, label, values]) => {
      const value = this.picks.get(key);
      const trigger = chip(`facet-${key}`, this.popover === key).rounded(S["radius-sm"]).border(1)
        .border_color(value ? t["action-soft"] : t["control-line"]).bg(value ? t["action-soft"] : t.surface).text_color(value ? t["action-text"] : t.text)
        .accessibility_label(value ? `${label}: ${value}. Clear` : `Filter by ${label.toLowerCase()}`)
        .child(value ?? label).child(svg(ICON(value ? "x" : "chevron-down")).size(12).text_color(value ? t["action-text"] : t["text-3"]));
      if (value) return trigger.on_click((_event, cx) => { this.picks.delete(key); cx.notify(); });
      /** @type {Map<string, number>} */
      const counts = new Map();
      for (const run of views[this.view]) for (const name of values(run)) counts.set(name, (counts.get(name) ?? 0) + 1);
      return Popover.new(`facet-${key}-menu`).trigger(trigger).anchor("top_left").open(this.popover === key)
        .on_open_change((/** @type {boolean} */ next, /** @type {import("gpui-kit").Context} */ cx) => { this.popover = next ? key : null; cx.notify(); })
        .content(v_flex().min_w(200).p(4).gap(2).rounded(S["radius-md"]).bg(t.surface).border(1).border_color(t["control-line"])
          .children([...counts].map(([name, count]) => Button.new(`pick-${key}-${name}`).h(S["control-fine"]).px(8).flex().items_center().gap(8)
            .rounded(S["radius-sm"]).text_size(S["type-small"][0]).hover((s) => s.bg(t.raised)).focus((s) => s.bg(t.raised))
            .on_click((_event, cx) => { this.picks.set(key, name); this.popover = null; cx.notify(); })
            .child(div().flex_1().child(name)).child(div().text_color(t["text-3"]).child(String(count))))));
    };
    return v_flex().flex_none().px(S["space-4"]).py(S["space-2"]).gap(S["space-2"]).border_b(1).border_color(t.line)
      .child(h_flex().gap(6).flex_wrap().children(RUN_VIEWS.map((id) => {
        const on = this.view === id;
        return chip(`view-${id}`, on).rounded(S["radius-full"]).font_weight(on ? 600 : 500).bg(on ? t.emph : t.surface).text_color(on ? t["on-emph"] : t.text)
          .on_click((_event, cx) => { [this.view, this.popover] = [id, null]; cx.notify(); })
          .child(LABELS[id]).child(div().opacity(0.7).child(String(views[id].length)));
      })))
      .child(h_flex().gap(6).flex_wrap().children(RUN_FACETS.map(picker)));
  }

  /** A run row: task key, step, state and start; agent and model; run, outcome, duration and tokens. @param {any} run @param {Palette} t @param {number} now */
  row(run, t, now) {
    const state = runState(run);
    const tokens = tokenText(run.tokens);
    const facts = [`run ${run.run}`, run.outcome, duration(run, now), tokens && `${tokens} tokens`].filter(Boolean).join(" · ");
    return Button.new(`run-${run.run}`).w_full().min_h(S["pane-row"]).px(S["space-2"]).py(S["space-2"]).flex().gap(S["space-3"]).items_start()
      .rounded(S["radius-sm"]).hover((s) => s.bg(t.raised)).focus((s) => s.border(2).border_color(t["action-text"]))
      .accessibility_label(`Open task ${run.key}`)
      .on_click(() => setShared("priory.task.open", { task: run.task, at: Date.now() }))
      .child(this.tile(state, t))
      .child(v_flex().flex_1().min_w_0().gap(2)
        .child(h_flex().gap(6).items_baseline().text_size(S["type-caption"][0]).text_color(t["text-2"])
          .child(div().font_family(this.font.mono).text_color(t.text).child(run.key))
          .child(run.step)
          .child(div().text_color(t[ROLE[state] ?? "text-2"]).child(state))
          .child(div().flex_1()).child(div().text_color(t["text-3"]).child(ago(run.started, now))))
        .child(div().min_w_0().truncate().text_size(S["type-small"][0]).child([run.agent, run.model].filter(Boolean).join(", ")))
        .child(div().min_w_0().truncate().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(facts)));
  }

  /** @param {string} state @param {Palette} t */
  tile(state, t) {
    const [fill, ink, icon] = TILES[state] ?? ["tile-muted", "on-tile", "circle-help"];
    return h_flex().size(S["pane-tile"]).flex_none().items_center().justify_center().rounded(S["radius-tile"]).bg(t[fill]).accessibility_label(state)
      .child(icon ? svg(ICON(icon)).size(14).text_color(t[ink]) : div().size(8).rounded(999).bg(t[ink]));
  }
}
