import { View, div, svg } from "gpui-kit";
import { h_flex, v_flex, Button, Popover } from "gpui-base";
import { appHome, exec, home as herdrHome, readTextFile, setShared } from "herdr";
import { follow, priory } from "./priory.js";
import { FONTS, palette, STRUCTURE as S } from "./tokens.js";
import { elsewhere, filtered, remoteActs, remoteSent, TASK_VIEWS, taskFacets, taskViews } from "./view.js";

const RETRY_MS = 3000;
// How long a first press on Cancel stays armed for the second, as an ask's command option.
const ARM_MS = 4000;
const ACTS = /** @type {Record<string, string>} */ ({ resume: "Resume", cancel: "Cancel", release: "Release" });
const TONES = /** @type {Record<string, string>} */ ({ action: "action-text", warn: "warn-text", quiet: "text-3" });
const ICON = (/** @type {string} */ name) => `mods/priory-tasks/icons/${name}.svg`;
const LABELS = /** @type {Record<string, string>} */ ({ active: "Active", blocked: "Blocked", ended: "Ended" });
const EMPTY = /** @type {Record<string, string>} */ ({
  active: "No task is running or waiting. Add one from the Desk or the Queue.",
  blocked: "No task is blocked.",
  ended: "No task has ended yet.",
});
// A task state's tile: its fill, its ink and its icon, as the Task panel's; null draws a dot.
/** @type {Record<string, [string, string, string | null]>} */
const TILES = {
  queued: ["tile-muted", "on-tile", "circle-dashed"], held: ["tile-hold", "on-tile", "pause"], ready: ["tile-live", "on-tile-live", "circle-dot"],
  running: ["tile-live", "on-tile-live", null], asleep: ["tile-muted", "on-tile", "moon"], blocked: ["tile-warn", "on-tile", "hand"],
  done: ["tile-ok", "on-tile", "check"], failed: ["tile-danger", "on-tile", "circle-x"], dropped: ["tile-muted", "on-tile", "minus"],
  elsewhere: ["tile-muted", "on-tile", "server"],
};
// A task state's text color (DESIGN.md, State tile); the other states take --text-2.
const ROLE = /** @type {Record<string, string>} */ ({ running: "action-text", ready: "action-text", blocked: "warn-text", failed: "danger-text", done: "ok-text" });
// The states whose row shows the reason under its title (DESIGN.md, Task row and run row).
const REASONED = ["blocked", "held", "failed", "dropped"];

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

/** How long a task has been in its state, as the web's age(): "18 min", "4 h", then a date. @param {string} since @param {number} now */
function age(since, now) {
  const minutes = Math.max(0, Math.round((now - Date.parse(since)) / 60000));
  if (minutes < 60) return `${minutes} min`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)} h`;
  return new Date(since).toLocaleString("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
}

export default class PrioryTasks extends View {
  /** @param {unknown} _props @param {import("gpui-kit").AsyncContext} cx */
  init(_props, cx) {
    /** @type {Awaited<ReturnType<typeof connect>> | null} */
    this.priory = null;
    /** @type {any[] | null} `priory tasks --json`'s tasks */
    this.tasks = null;
    /** @type {string | null} */
    this.error = null;
    this.view = "active";
    /** @type {Map<string, string>} each picked facet's value */
    this.picks = new Map();
    /** @type {string | null} the facet whose menu is open */
    this.popover = null;
    this.font = FONTS.indigo;
    /** @type {string | null} the `runner:key` of the remote row opened in place */
    this.open = null;
    /** @type {{ act: string, at: number, target: string, generation: number } | null} a Cancel armed by a first press, for the generation it showed */
    this.armed = null;
    this.sending = false;
    /** @type {{ target: string, text: string } | null} why the last send failed, and to which task */
    this.sendError = null;
    /** @type {ReturnType<typeof priory>} the client of the last read, which its wait uses */
    let client;
    cx.spawn((cx) => follow({
      read: async () => {
        this.priory ??= await connect();
        client = this.priory.client;
        const read = await client.tasks();
        [this.tasks, this.error] = [read.tasks, null];
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
    const facets = taskFacets(this.tasks ?? []);
    const views = /** @type {Record<string, any[]>} */ (taskViews(filtered(this.tasks ?? [], this.picks, facets)));
    const shown = views[this.view];
    return v_flex().id("priory-tasks").size_full().min_h_0().bg(t.bg).text_color(t.text).font_family(this.font.ui).text_size(S["type-body"][0])
      .child(h_flex().flex_none().h(S["pane-head"]).px(S["space-4"]).items_center().border_b(1).border_color(t.line).child(div().font_weight(650).child("Tasks")))
      .when(Boolean(this.error), (el) => el.child(div().flex_none().px(S["space-4"]).py(6).whitespace_normal().text_size(S["type-small"][0])
        .text_color(t["warn-text"]).border_b(1).border_color(t.line)
        .child(`Could not read the tasks: ${this.error}. Check that priory runs, then it reads again on its own.`)))
      .child(this.filters(views, facets, t))
      .child(v_flex().id("tasks-body").flex_1().min_h_0().overflow_y_scrollbar().px(S["space-2"]).py(S["space-2"]).children(this.body(shown, t)));
  }

  /** @param {any[]} shown @param {Palette} t */
  body(shown, t) {
    const note = (/** @type {string} */ line) => [div().p(S["space-2"]).whitespace_normal().text_color(t["text-2"]).child(line)];
    if (!this.tasks) return this.error ? [] : note("Reading the tasks…");
    if (!shown.length) return note(this.picks.size ? "No task matches these filters." : EMPTY[this.view]);
    const now = Date.now();
    return shown.map((task) => this.row(task, t, now));
  }

  /** The saved views with their filtered counts, then a chip per facet offering every task's values A to Z. @param {Record<string, any[]>} views @param {any[]} facets @param {Palette} t */
  filters(views, facets, t) {
    const chip = (/** @type {string} */ id, /** @type {boolean} */ on) => Button.new(id).h(S["pane-chip"]).px(10).flex().flex_none().items_center().gap(4)
      .selected(on).text_size(S["type-small"][0]).focus((s) => s.border(2).border_color(t["action-text"]));
    /** @param {[string, string, (task: any) => string[], unknown, ((name: string) => string)?]} facet */
    const picker = ([key, label, values, , named = (name) => name]) => {
      const value = this.picks.get(key);
      const trigger = chip(`facet-${key}`, this.popover === key).rounded(S["radius-sm"]).border(1)
        .border_color(value ? t["action-soft"] : t["control-line"]).bg(value ? t["action-soft"] : t.surface).text_color(value ? t["action-text"] : t.text)
        .accessibility_label(value ? `${label}: ${value}. Clear` : `Filter by ${label.toLowerCase()}`)
        .child(value ?? label).child(svg(ICON(value ? "x" : "chevron-down")).size(12).text_color(value ? t["action-text"] : t["text-3"]));
      if (value) return trigger.on_click((_event, cx) => { this.picks.delete(key); cx.notify(); });
      /** @type {Map<string, number>} */
      const counts = new Map();
      for (const task of this.tasks ?? []) for (const name of values(task)) counts.set(name, (counts.get(name) ?? 0) + 1);
      return Popover.new(`facet-${key}-menu`).trigger(trigger).anchor("top_left").open(this.popover === key)
        .on_open_change((/** @type {boolean} */ next, /** @type {import("gpui-kit").Context} */ cx) => { this.popover = next ? key : null; cx.notify(); })
        .content(v_flex().min_w(200).p(4).gap(2).rounded(S["radius-md"]).bg(t.surface).border(1).border_color(t["control-line"])
          .children([...counts].sort(([a], [b]) => a.localeCompare(b)).map(([name, count]) => Button.new(`pick-${key}-${name}`).h(S["control-fine"]).px(8).flex().items_center().gap(8)
            .rounded(S["radius-sm"]).text_size(S["type-small"][0]).hover((s) => s.bg(t.raised)).focus((s) => s.bg(t.raised))
            .on_click((_event, cx) => { this.picks.set(key, name); this.popover = null; cx.notify(); })
            .child(div().flex_1().child(named(name))).child(div().text_color(t["text-3"]).child(String(count))))));
    };
    return v_flex().flex_none().px(S["space-4"]).py(S["space-2"]).gap(S["space-2"]).border_b(1).border_color(t.line)
      .child(h_flex().gap(6).flex_wrap().children(TASK_VIEWS.map((id) => {
        const on = this.view === id;
        return chip(`view-${id}`, on).rounded(S["radius-full"]).font_weight(on ? 600 : 500).bg(on ? t.emph : t.surface).text_color(on ? t["on-emph"] : t.text)
          .on_click((_event, cx) => { [this.view, this.popover] = [id, null]; cx.notify(); })
          .child(LABELS[id]).child(div().opacity(0.7).child(String(views[id].length)));
      })))
      .child(h_flex().gap(6).flex_wrap().children(facets.map(picker)));
  }

  /** The desk's task row: key, workflow, step, state and age; the title; lane, band and batch, or the reason. @param {any} task @param {Palette} t @param {number} now */
  row(task, t, now) {
    // As the web's ago(): "12 min ago", or "on Nov 15, 12:00" past a day.
    const away = elsewhere(task, (iso) => (/ (min|h)$/.test(age(iso, now)) ? `${age(iso, now)} ago` : `on ${age(iso, now)}`));
    if (away) return this.remoteRow(task, away, t, now);
    const reason = REASONED.includes(task.state) && task.reason;
    const caption = (/** @type {string} */ line, /** @type {string} */ color) => div().min_w_0().truncate().text_size(S["type-caption"][0]).text_color(t[color]).child(line);
    return Button.new(`task-${task.id}`).w_full().min_h(S["pane-row"]).px(S["space-2"]).py(S["space-2"]).flex().gap(S["space-3"]).items_start()
      .rounded(S["radius-sm"]).hover((s) => s.bg(t.raised)).focus((s) => s.border(2).border_color(t["action-text"]))
      .accessibility_label(`Open task ${task.key}`)
      .on_click(() => setShared("priory.task.open", { task: task.id, at: Date.now() }))
      .child(this.tile(task.state, t))
      .child(v_flex().flex_1().min_w_0().gap(2)
        .child(h_flex().gap(6).items_baseline().text_size(S["type-caption"][0]).text_color(t["text-2"])
          .child(div().font_family(this.font.mono).text_color(t.text).child(task.key))
          .child(task.workflow).child(task.step)
          .child(div().text_color(t[ROLE[task.state] ?? "text-2"]).child(task.state))
          .child(div().flex_1()).child(div().text_color(t["text-3"]).child(age(task.since, now))))
        .child(div().min_w_0().truncate().text_size(S["type-small"][0]).child(task.title || task.tldr || ""))
        .child(h_flex().gap(6).min_w_0()
          .child(reason ? caption(reason, ROLE[task.state] ?? "text-2") : caption([task.lane, task.band].filter(Boolean).join(" · "), "text-3"))
          .when(task.batch != null, (el) => el.child(caption(`· batch ${task.batch}`, "text-3")))));
  }

  /**
   * Another runner's task: its hold for a state, where it runs, and the newest action sent to it.
   * Pressing it opens its actions in place, since its Task panel stays on its runner.
   * @param {any} task @param {string} away @param {Palette} t @param {number} now
   */
  remoteRow(task, away, t, now) {
    const target = `${task.runner}:${task.key}`;
    const sent = remoteSent(task);
    const caption = (/** @type {string} */ line, /** @type {string} */ color) => div().min_w_0().truncate().text_size(S["type-caption"][0]).text_color(t[color]).child(line);
    const row = Button.new(`remote-${target}`).w_full().min_h(S["pane-row"]).px(S["space-2"]).py(S["space-2"]).flex().gap(S["space-3"]).items_start()
      .rounded(S["radius-sm"]).hover((s) => s.bg(t.raised)).focus((s) => s.border(2).border_color(t["action-text"]))
      .accessibility_label(this.open === target ? `Close the actions of ${target}` : `Open the actions of ${target}`)
      .on_click((_event, cx) => { [this.open, this.armed, this.sendError] = [this.open === target ? null : target, null, null]; cx.notify(); })
      .child(this.tile("elsewhere", t))
      .child(v_flex().flex_1().min_w_0().gap(2)
        .child(h_flex().gap(6).items_baseline().text_size(S["type-caption"][0]).text_color(t["text-2"])
          .child(div().font_family(this.font.mono).text_color(t.text).child(task.key))
          .child(task.workflow ?? "").child(task.hold)
          .child(div().flex_1()).child(div().text_color(t["text-3"]).child(age(task.since, now))))
        .child(div().min_w_0().truncate().text_size(S["type-small"][0]).child(task.title || task.key))
        .when(Boolean(sent), (el) => el.child(caption(sent?.text ?? "", TONES[sent?.tone ?? "quiet"])))
        .child(caption(away, "text-3")))
      .child(svg(ICON(this.open === target ? "chevron-up" : "chevron-down")).size(14).flex_none().text_color(t["text-3"]));
    if (this.open !== target) return row;
    return v_flex().w_full().child(row).child(v_flex().pl(S["pane-tile"] + S["space-3"] + S["space-2"]).pr(S["space-2"]).pb(S["space-2"]).gap(S["space-2"])
      .child(h_flex().gap(6).flex_wrap().children(remoteActs(task).map((act) => this.remoteButton(task, target, act, t, now))))
      .when(this.sendError?.target === target, (el) => el.child(div().whitespace_normal().text_size(S["type-caption"][0]).text_color(t["warn-text"]).child(this.sendError?.text ?? "")))
      .child(div().whitespace_normal().text_size(S["type-caption"][0]).text_color(t["text-2"])
        .child(`This task runs on ${task.runner}. An action goes there through the log, and ${task.runner} applies it when it reads it.`)));
  }

  /** One action of an open remote row; Cancel arms on the first press. @param {any} task @param {string} target @param {string} act @param {Palette} t @param {number} now */
  remoteButton(task, target, act, t, now) {
    const armed = this.armedFor(task, target) && this.armed?.act === act;
    const left = armed && this.armed ? Math.max(0, 1 - (now - this.armed.at) / ARM_MS) : 0;
    return Button.new(`remote-${target}-${act}`).h(S["control-fine"]).px(10).flex().flex_col().justify_center().rounded(S["radius-sm"])
      .bg(armed ? t.emph : t.surface).text_color(armed ? t["on-emph"] : t.text).border(armed ? 3 : 1).border_color(armed ? t.action : t["control-line"])
      .disabled(this.sending).opacity(this.sending ? 0.45 : 1).focus((s) => s.border(2).border_color(t["action-text"]))
      .accessibility_label(armed ? `Press again to ${act} ${target}` : `${ACTS[act]} ${target}`)
      .on_click((_event, cx) => this.remoteAct(task, target, act, cx))
      .child(div().text_size(S["type-small"][0]).font_weight(600).child(armed ? `Press again to ${act}` : ACTS[act]))
      .when(armed, (el) => el.child(div().h(3).w_full().rounded(2).bg(t["on-emph-2"]).child(div().h(3).w(`${Math.round(left * 100)}%`).rounded(2).bg(t.action))));
  }

  /** @param {any} task @param {string} target @param {string} act @param {import("gpui-kit").Context} cx */
  remoteAct(task, target, act, cx) {
    if (this.sending) return;
    if (act === "cancel" && !this.armedFor(task, target)) {
      this.armed = { act, at: Date.now(), target, generation: task.generation };
      this.drain(cx);
      return cx.notify();
    }
    [this.armed, this.sending, this.sendError] = [null, true, null];
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        await (this.priory ??= await connect()).client.remote(act, target, task.generation);
      } catch (error) {
        this.sendError = { target, text: `Could not ${act}: ${message(error)}` };
      }
      this.sending = false;
      cx.notify();
    });
  }

  /** Whether a Cancel armed on this task still holds: a new generation is a task the person did not see. @param {any} task @param {string} target */
  armedFor(task, target) {
    return this.armed?.target === target && this.armed.generation === task.generation;
  }

  // Redraws the draining bar until the arming lapses, then shows Cancel unarmed.
  /** @param {import("gpui-kit").Context} cx */
  drain(cx) {
    cx.timer.after(100, (cx) => {
      if (!this.armed) return;
      if (Date.now() - this.armed.at >= ARM_MS) this.armed = null;
      else this.drain(cx);
      cx.notify();
    });
  }

  /** @param {string} state @param {Palette} t */
  tile(state, t) {
    const [fill, ink, icon] = TILES[state] ?? ["tile-muted", "on-tile", "circle-help"];
    return h_flex().size(S["pane-tile"]).flex_none().items_center().justify_center().rounded(S["radius-tile"]).bg(t[fill]).accessibility_label(state)
      .child(icon ? svg(ICON(icon)).size(14).text_color(t[ink]) : div().size(8).rounded(999).bg(t[ink]));
  }
}
