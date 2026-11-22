import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, Button, TextView } from "gpui-base";
import { appHome, checking, exec, focused, home as herdrHome, layout as shown, nextCommand, panes, readTextFile, run, setShared, settings } from "herdr";
import { showAsks } from "./layout.js";
import { follow, priory, spansHtml } from "./priory.js";
import { selectable, stateTile } from "./ui.js";
import { addChoices } from "./add.js";
import { AddForm } from "./add-form.js";
import { deskModel, mergeSince, tomorrowAt } from "./view.js";
import { FONTS, palette, STRUCTURE as S } from "./tokens.js";

// DESIGN.md, Herdr adapter, Desk: from 1024 px the count column sits beside the wide table.
const WIDE = 1024;
const COLUMN = 440;
const RETRY_MS = 5000;
const SKELETON_MS = 300;
const ICONS = "mods/priory-desk/icons";
const STEPS = /** @type {Record<string, number>} */ ({ j: 1, down: 1, k: -1, up: -1 });
// Each kind's state tile (DESIGN.md, State tile): its fill token, its icon, and its reason's text color.
const TILES = /** @type {Record<string, [string, string | null, string]>} */ ({
  ask: ["wait", "ask", "text-2"],
  todo: ["wait", "square", "text-2"],
  memories: ["wait", "brain", "text-2"],
  blocked: ["tile-warn", "hand", "warn-text"],
  prompt: ["tile-warn", "hand", "warn-text"],
  failed: ["tile-danger", "circle-x", "danger-text"],
  delivery: ["tile-danger", "circle-x", "danger-text"],
  held: ["tile-hold", "pause", "text-2"],
  batch: ["tile-muted", "layers", "text-2"],
  queued: ["tile-muted", "circle-dashed", "text-2"],
  ready: ["tile-live", "circle-dot", "action-text"],
  running: ["tile-live", null, "action-text"],
  asleep: ["tile-muted", "moon", "text-2"],
  done: ["tile-ok", "check", "ok-text"],
  report: ["tile-ok", "check", "text-2"],
  dropped: ["tile-muted", "minus", "text-2"],
});
const RESTART = /** @type {Record<string, string>} */ ({ blocked: "Resume", prompt: "Resume", failed: "Revive" });
const HOUR = { for: "1h" };
// DESIGN.md, Ask detail: an armed Accept waits this long for the second press.
const ARM_MS = 4000;
/**
 * Text with its autolinks; plain text stays a plain child. Linked text keeps its press, so a link
 * opens without selecting the row around it.
 * @param {string} id @param {{ text: string, url?: string }[]} spans
 */
const linked = (id, spans) => (spans.some((span) => span.url)
  ? div().on_mouse_down("left", (/** @type {unknown} */ _event, /** @type {import("gpui-kit").Context} */ cx) => cx.stop_propagation()).child(TextView.html(id, spansHtml(spans)))
  : spans.map((span) => span.text).join(""));
// Rows and the next thing select on a press, so selectable text keeps its press to itself.
/** @param {string} id @param {string | { text: string, url?: string }[]} text */
const copyable = (id, text) => div().on_mouse_down("left", (/** @type {unknown} */ _event, /** @type {import("gpui-kit").Context} */ cx) => cx.stop_propagation()).child(selectable(id, text));
const MUTED_TILE = /** @type {[string, string, string]} */ (["tile-muted", "circle-dashed", "text-2"]);

/** @typedef {ReturnType<typeof deskModel>} Model */
/** @typedef {ReturnType<Model["sections"]>[number]["rows"][number] & { count?: number }} Row */

// settings.json's "priory" object: { binary, machine, factory, theme }, each optional, as the Asks panel reads it.
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
  if (!home.startsWith("/")) throw new Error(`no home folder on ${on ?? "this machine"}`);
  if (binary.startsWith("~/")) binary = home + binary.slice(1);
  const client = priory({ exec, binary, on, factory: config.factory ?? null });
  // Links are optional: without a factory priory can read, the panel shows its text unlinked.
  const links = await client.links().then((read) => read.links, () => []);
  return { theme: config.theme ?? "graphite", client, links };
}

/** @param {Ui} ui @param {string | undefined} text */
const routed = (ui, text) => div().text_size(S["type-caption"][0]).text_color(ui.t["action-text"]).child(text ?? "");

/** @param {unknown} error */
const message = (error) => (error instanceof Error ? error.message : String(error));

export default class PrioryDesk extends View {
  /** @param {unknown} _props @param {import("gpui-kit").Context} cx */
  init(_props, cx) {
    /** @type {{ desk: any, asks: any, tasks: any, queue: any, memories: any } | null} */
    this.data = null;
    /** @type {{ pattern: string, url: string }[]} */
    this.links = [];
    /** @type {any[]} */
    this.since = [];
    /** @type {string | null} */
    this.sinceFrom = null;
    this.error = /** @type {string | null} */ (null);
    this.actionError = /** @type {string | null} */ (null);
    this.view = "needs";
    this.selected = /** @type {string | null} */ (null);
    /** @type {number} */
    this.width = 0;
    this.slow = false;
    this.busy = false;
    // A memory candidate's Accept armed by its first press, and the candidates whose current text shows.
    /** @type {{ id: string, at: number } | null} */
    this.memoryArmed = null;
    /** @type {Set<string>} */
    this.currentShown = new Set();
    // Candidates whose verdict was sent, with what priory said: their buttons stay off until the desk drops them.
    /** @type {Map<string, string>} */
    this.memorySent = new Map();
    /** @type {Promise<{ theme: string, client: ReturnType<typeof priory> }> | null} */
    this.client = null;
    this.theme = "graphite";
    this.chain = 0;
    this.reading = false;
    /** @type {string | null} */
    this.opened = null;
    this.seenSent = false;
    /** @type {string | null} the key `priory add` returned, selected once a read lists it */
    this.addedKey = null;
    this.adding = new AddForm(async () => (await (this.client ??= connect())).client, (key) => {
      this.view = "all";
      this.addedKey = key;
    });
    this.focus = cx.focus_handle();
    // herdr-app check runs init for real: no priory call, and above all no `priory seen`.
    if (checking()) return;
    // With "focusable" in mod.json, pane.go moves onto the panel and sends "focus".
    cx.spawn(async (cx) => {
      for (;;) {
        if ((await nextCommand()) === "focus") this.focus.focus();
        cx.notify();
      }
    });
    cx.timer.after(SKELETON_MS, (cx) => { this.slow = true; cx.notify(); });
    this.load(cx);
  }

  // A snooze ends by the clock, which no revision records, so every `priory wait` ends in a read.
  // One loop runs at a time: Retry starts a new one, and an older loop stops at its next step.
  /** Reads every listing, then waits and reads again. @param {import("gpui-kit").Context} cx */
  load(cx) {
    const chain = ++this.chain;
    /** @type {ReturnType<typeof priory>} the client of the last read, which its wait uses */
    let client;
    cx.spawn((cx) => follow({
      read: async () => {
        // Connects on first use, and again after a failure, so a machine that comes back works.
        this.client ??= connect().catch((error) => { this.client = null; throw error; });
        ({ theme: this.theme, links: this.links, client } = await this.client);
        this.reading = true;
        // Taken before the first read, so an event between that read and `priory seen` still shows next time.
        const read = new Date().toISOString().replace(/\.\d+Z$/, "Z");
        const [desk, asks, tasks, queue] = await Promise.all([client.desk({ all: true }), client.asks(), client.tasks(), client.queue()])
          .finally(() => { this.reading = false; });
        // Only `priory memories DIR` says what memory each candidate replaces, so it is read while one waits.
        const memories = desk.items.some((/** @type {any} */ item) => item.kind === "memories") ? await client.memories() : null;
        if (chain !== this.chain) return null;
        if (!this.data) {
          this.sinceFrom = desk.last_visit;
          this.opened = read;
        }
        this.since = mergeSince(this.since, desk.since_you_looked);
        this.data = { desk, asks, tasks, queue, memories };
        this.error = null;
        cx.notify();
        return desk.revision;
      },
      wait: (since) => client.wait(since),
      sleep: (ms) => cx.sleep(ms),
      failed: (error) => {
        this.error = message(error);
        cx.notify();
      },
      retry: RETRY_MS,
      stopped: () => chain !== this.chain,
    }));
  }

  // Records the visit once Since you looked is drawn, which happens only from WIDE.
  /** @param {import("gpui-kit").Context} cx */
  seen(cx) {
    if (this.seenSent || !this.opened || !this.client) return;
    this.seenSent = true;
    const client = this.client;
    const at = this.opened;
    cx.spawn(async (cx) => {
      try {
        await (await client).client.seen({ at });
      } catch (error) {
        this.actionError = `Could not record your visit: ${message(error)}`;
        cx.notify();
      }
    });
  }

  /** Runs one Priory verb; the next read shows its result. @param {import("gpui-kit").Context} cx @param {string} verb @param {(client: ReturnType<typeof priory>) => Promise<unknown>} act */
  act(cx, verb, act) {
    if (this.busy) return;
    this.busy = true;
    this.actionError = null;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        await act((await (this.client ??= connect())).client);
      } catch (error) {
        this.client = null;
        this.actionError = `Could not ${verb}: ${message(error)}`;
      }
      this.busy = false;
      cx.notify();
    });
  }

  /** @param {import("gpui-kit").Context} cx @param {{ kind: string, id: number | string }} item @param {{ for?: string, until?: string } | null} when null unsnoozes */
  snooze(cx, item, when) {
    this.act(cx, when ? "snooze" : "unsnooze", (client) => client.snooze(item.kind, item.id, when));
  }

  // Resume for a blocked task or an uncertain prompt, Revive for a failed one: the row's recommendation.
  /** @param {import("gpui-kit").Context} cx @param {Row} row */
  restart(cx, row) {
    if (row.task == null || !RESTART[row.kind]) return;
    const task = row.task;
    this.act(cx, RESTART[row.kind].toLowerCase(), (client) => (row.kind === "failed" ? client.revive(task) : client.resume(task)));
  }

  // Done for a to-do, as `priory todo done`.
  /** @param {import("gpui-kit").Context} cx @param {number} todo */
  done(cx, todo) {
    this.act(cx, "mark it done", (client) => client.todoDone(todo));
  }

  // Read for a report, as `priory inbox read`, so its row leaves the desk.
  /** @param {import("gpui-kit").Context} cx @param {number} seq */
  read(cx, seq) {
    this.act(cx, "mark it read", (client) => client.read(seq));
  }

  // Accept arms on the first press and publishes on the second, within ARM_MS; Drop publishes at once.
  /** @param {import("gpui-kit").Context} cx @param {string} id @param {boolean} accept */
  decide(cx, id, accept) {
    const armed = this.memoryArmed?.id === id && Date.now() - this.memoryArmed.at < ARM_MS;
    this.memoryArmed = null;
    if (accept && !armed) {
      this.memoryArmed = { id, at: Date.now() };
      cx.timer.after(ARM_MS, (cx) => {
        if (this.memoryArmed?.id === id && Date.now() - this.memoryArmed.at >= ARM_MS) this.memoryArmed = null;
        cx.notify();
      });
      return cx.notify();
    }
    this.act(cx, accept ? "accept it" : "drop it", async (client) => {
      const said = await (accept ? client.acceptMemory(id) : client.dropMemory(id));
      this.memorySent.set(id, `${said.trim()}, then it leaves this list.`);
    });
  }

  /** @param {number | string} ask */
  answer(ask) {
    setShared("priory.asks.open", { ask, at: Date.now() });
    showAsks(shown(), panes() ?? [], focused()?.space).forEach(([command, args]) => run(command, args));
  }

  /** @param {number} task */
  attach(task) {
    setShared("priory.attach", { task, at: Date.now() });
  }

  // The Priory bar opens the Task tab, and the Task panel shows the task.
  /** @param {number} task */
  openTask(task) {
    setShared("priory.task.open", { task, at: Date.now() });
  }

  /** @param {Model} model */
  rows(model) {
    return model.sections(this.view).flatMap((section) => section.rows);
  }

  /** @param {import("gpui-kit").KeyEvent} event @param {import("gpui-kit").Context} cx @param {Model} model */
  key(event, cx, model) {
    if (event.keystroke === "escape" && this.adding.open) {
      this.adding.hide(cx);
      this.focus.focus();
      return cx.stop_propagation();
    }
    if (this.adding.typing) return;
    if (event.keystroke === "n") {
      this.adding.show(cx);
      return cx.stop_propagation();
    }
    const rows = this.rows(model);
    const index = rows.findIndex((row) => row.id === this.selected);
    const row = rows[index];
    const step = STEPS[event.keystroke];
    if (step && rows.length) {
      this.selected = rows[Math.min(rows.length - 1, Math.max(0, index + step))].id;
    } else if (event.keystroke === "enter" && row?.ask != null && !row.snoozed) {
      this.answer(row.ask);
    } else if (event.keystroke === "enter" && row?.id === "asleep") {
      this.view = "asleep";
    } else if (event.keystroke === "s" && row?.snooze) {
      this.snooze(cx, row.snooze, row.snoozed ? null : HOUR);
    } else if (event.keystroke === "r" && row && RESTART[row.kind]) {
      this.restart(cx, row);
    } else {
      return;
    }
    cx.stop_propagation();
    cx.notify();
  }

  /** @param {import("gpui-kit").Context} cx */
  render(cx) {
    // Every token but --scrim is a hex color.
    const t = /** @type {Record<string, import("gpui-kit").Color>} */ (palette(this.theme, window.appearance()));
    const ui = new Ui(t, FONTS[/** @type {keyof typeof FONTS} */ (this.theme)] ?? FONTS.graphite);
    const root = v_flex().id("priory-desk").size_full().min_h_0().bg(t.bg).text_color(t.text).font_family(ui.fonts.ui).text_size(S["type-body"][0])
      .track_focus(this.focus)
      .on_size_change((/** @type {{ width: number }} */ size, /** @type {import("gpui-kit").Context} */ cx) => {
        // on_size_change reports zoomed pixels.
        const width = size.width / Number(settings()?.zoom || 1);
        if (width === this.width) return;
        this.width = width;
        cx.notify();
      });
    const failed = this.error
      ? h_flex().flex_none().px(S["space-4"]).py(S["space-2"]).gap(S["space-2"]).items_center().border_b(1).border_color(t.line).bg(t.surface)
        .child(div().flex_1().min_w_0().whitespace_normal().text_size(S["type-small"][0]).text_color(t["danger-text"])
          .child(copyable("desk-error", `Could not read the desk: ${this.error}. Check that priory runs where settings.json's "priory" object says, then retry.`)))
        .child(ui.button("retry", "Retry", (_e, cx) => {
          if (this.reading) return;
          this.load(cx);
        }, { disabled: this.reading }))
      : null;
    if (!this.data) {
      return root.when(Boolean(failed), (el) => el.child(failed))
        .when(!this.error && this.slow, (el) => el.child(v_flex().p(S["space-4"]).gap(S["space-2"])
          .children([0, 1, 2].map((i) => div().id(`skeleton-${i}`).h(S["pane-row"]).rounded(S["radius-md"]).bg(t.raised)))));
    }
    const data = { ...this.data, desk: { ...this.data.desk, since_you_looked: this.since, last_visit: this.sinceFrom } };
    const model = deskModel(data, { now: Date.now(), links: this.links });
    const rows = this.rows(model);
    const added = this.addedKey && rows.find((row) => row.key === this.addedKey);
    if (added) [this.selected, this.addedKey] = [added.id, null];
    if (!rows.some((row) => row.id === this.selected)) this.selected = rows[0]?.id ?? null;
    const wide = this.width >= WIDE;
    if (wide) this.seen(cx);
    // The wide table needs --pane-split beside the column, or its question column has no room.
    const table = wide && this.width - COLUMN >= S["pane-split"];
    const summary = v_flex().gap(S["space-3"]).flex_none()
      .child(this.count(ui, model))
      .when(this.adding.open, (el) => el.child(this.adding.render(t, ICONS, addChoices(this.data.queue, this.data.tasks.tasks))))
      .when(Boolean(model.next), (el) => el.child(this.next(ui, model, wide)))
      .children(model.banners.map((text, i) => ui.banner(`banner-${i}`, text)))
      .when(Boolean(this.actionError), (el) => el.child(div().whitespace_normal().text_size(S["type-small"][0]).text_color(t["danger-text"]).child(copyable("action-error", String(this.actionError)))));
    const list = v_flex().gap(S["space-3"]).min_w_0()
      .child(this.views(ui, model))
      .children(model.sections(this.view).map((section) => this.section(ui, section, table)))
      .when(Boolean(model.empty) && this.view === "needs", (el) => el.child(ui.note(String(model.empty))))
      .when(!wide, (el) => el.child(this.upNext(ui, model)));
    const body = wide
      ? h_flex().flex_1().min_h_0().items_stretch()
        .child(v_flex().id("desk-column").w(COLUMN).flex_none().min_h_0().overflow_y_scrollbar().p(S["space-4"]).gap(S["space-4"]).border_r(1).border_color(t.line)
          .child(summary).child(this.upNext(ui, model)).child(this.sinceYouLooked(ui, model)))
        .child(v_flex().flex_1().min_w_0().min_h_0()
          .child(v_flex().id("desk-table").flex_1().min_h_0().overflow_y_scrollbar().p(S["space-4"]).child(list))
          .child(ui.hints([["J", "K", "move"], ["Enter", "answer"], ["S", "snooze 1 h"], ["R", "resume or revive"], ["N", "add"]])))
      : v_flex().id("desk-scroll").flex_1().min_h_0().overflow_y_scrollbar().p(S["space-4"]).gap(S["space-4"]).child(summary).child(list);
    return root.on_key_down((/** @type {import("gpui-kit").KeyEvent} */ event, /** @type {import("gpui-kit").Context} */ cx) => this.key(event, cx, model))
      .when(Boolean(failed), (el) => el.child(failed)).child(body);
  }

  /** @param {Ui} ui @param {Model} model */
  count(ui, model) {
    const { t } = ui;
    return h_flex().gap(S["space-3"]).items_center()
      .child(div().flex_none().font_family(ui.fonts.display).text_size(S["type-pane-count"][0]).line_height(S["type-pane-count"][1]).font_weight(800).text_color(t.count).child(String(model.count.number)))
      .child(v_flex().min_w_0()
        .child(div().font_family(ui.fonts.display).text_size(S["type-hero"][0]).font_weight(650).child(model.count.label))
        .when(Boolean(model.count.line), (el) => el.child(div().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(model.count.line))));
  }

  /** @param {Ui} ui @param {Model} model @param {boolean} wide */
  next(ui, model, wide) {
    const { t } = ui;
    const next = /** @type {NonNullable<Model["next"]>} */ (model.next);
    return v_flex().id("next-thing").p(S["space-4"]).gap(S["space-2"]).rounded(S["radius-card"]).bg(t.surface).border(1).border_color(t.line)
      .child(h_flex().gap(S["space-2"]).items_center().text_size(S["type-caption"][0])
        .child(ui.tile("next", "ask", "small"))
        .child(div().font_weight(600).child("The next thing"))
        .child(ui.mono(next.key ?? ""))
        .child(div().text_color(t["text-2"]).child(next.step ?? ""))
        .child(div().flex_1())
        .child(div().text_color(t["text-3"]).child(`waits ${next.age}`)))
      .child(div().whitespace_normal().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(copyable("next-why", `First because ${next.why}`)))
      .when(Boolean(next.routed), (el) => el.child(routed(ui, next.routed)))
      .when(Boolean(next.runner), (el) => el.child(div().text_size(S["type-caption"][0]).text_color(next.away ? t["warn-text"] : t["text-2"]).child([next.runner, next.away].filter(Boolean).join(", "))))
      .when(Boolean(next.sent), (el) => el.child(div().text_size(S["type-caption"][0]).text_color(next.sent?.refused ? t["warn-text"] : t["action-text"]).child(next.sent?.text ?? "")))
      .child(div().whitespace_normal().font_family(ui.fonts.display).text_size(S["type-pane-question"][0]).line_height(S["type-pane-question"][1] / S["type-pane-question"][0]).font_weight(650).child(copyable("next-question", next.questionSpans)))
      .when(Boolean(next.recommendation || next.tradeoff), (el) => el.child(div().whitespace_normal().text_size(S["type-small"][0]).text_color(t["text-2"])
        .child(copyable("next-recommendation", next.recommendation ? `Recommends ${next.recommendation}. ${next.tradeoff ?? ""}` : String(next.tradeoff)))))
      .when(Boolean(next.progress?.nextAction), (el) => el.child(div().whitespace_normal().text_size(S["type-caption"][0]).text_color(t["text-3"])
        .child(copyable("next-action", `Next: ${next.progress?.nextAction}`))))
      .child(h_flex().gap(S["space-2"]).flex_wrap().items_center()
        .when(next.ask != null, (el) => el.child(ui.button("next-answer", "Answer…", () => this.answer(/** @type {number | string} */ (next.ask)), { primary: true })))
        .when(next.todo != null, (el) => el.child(ui.button("next-done", "Done", (_e, cx) => this.done(cx, /** @type {number} */ (next.todo)), { disabled: this.busy })))
        .when(next.task != null, (el) => el.child(ui.button("next-open", "Open task", () => this.openTask(/** @type {number} */ (next.task))))
          .child(ui.button("next-attach", "Attach", () => this.attach(/** @type {number} */ (next.task)))))
        .when(next.snooze != null, (el) => el.child(ui.button("next-snooze", "Snooze 1 h", (_e, cx) => this.snooze(cx, /** @type {{ kind: string, id: number | string }} */ (next.snooze), HOUR), { icon: "clock", disabled: this.busy })))
        .when(wide && next.ask != null, (el) => el.child(div().flex_1()).child(ui.hints([["Enter", "answer"], ["S", "snooze"]], true))))
      .on_click((/** @type {unknown} */ _e, /** @type {import("gpui-kit").Context} */ cx) => { this.selected = next.snooze ? `${next.snooze.kind}:${next.snooze.id}` : null; this.focus.focus(); cx.notify(); });
  }

  /** @param {Ui} ui @param {Model} model */
  views(ui, model) {
    const { t } = ui;
    return h_flex().gap(S["space-1"]).flex_wrap()
      .child(ui.button("add-open", "Add", (_e, cx) => this.adding.show(cx), { icon: "plus" }))
      .children(model.views.map((view) => {
      const on = view.id === this.view;
      return Button.new(`view-${view.id}`).selected(on).h(S["pane-chip"]).px(S["space-3"]).gap(S["space-1"]).flex_none().items_center().rounded(S["radius-md"])
        .text_size(S["type-caption"][0]).font_weight(on ? 600 : 500).bg(on ? t["action-soft"] : t.surface).text_color(on ? t.text : t["text-2"])
        .border(1).border_color(on ? t["action-soft"] : t.line)
        .hover((s) => s.bg(t.raised)).focus((s) => s.border_color(t.action))
        .on_click((/** @type {unknown} */ _e, /** @type {import("gpui-kit").Context} */ cx) => { this.view = view.id; this.selected = null; cx.notify(); })
        .child(view.label).child(div().font_family(ui.fonts.mono).text_color(t["text-3"]).child(String(view.count)));
    }));
  }

  /** @param {Ui} ui @param {{ title: string, note: string | null, rows: Row[] }} section @param {boolean} table */
  section(ui, section, table) {
    const { t } = ui;
    return v_flex().gap(S["space-1"])
      .child(h_flex().gap(S["space-2"]).text_size(S["type-caption"][0])
        .child(div().font_weight(600).child(section.title))
        .when(Boolean(section.note), (el) => el.child(div().text_color(t["text-3"]).child(String(section.note)))))
      .children(section.rows.map((row) => this.row(ui, row, table)));
  }

  /** @param {Ui} ui @param {Row} row @param {boolean} table */
  row(ui, row, table) {
    const { t } = ui;
    const selected = row.id === this.selected;
    const reasonColor = t[(TILES[row.kind] ?? MUTED_TILE)[2]];
    const meta = row.recommends ? `Recommends ${row.recommends}` : null;
    const snoozed = row.snoozed;
    // The selected row's text selects and copies; another row's is a label, so a press selects the row.
    const text = selected ? copyable(`text-${row.id}`, row.textSpans) : linked(`text-${row.id}`, row.textSpans);
    const note = row.note
      ? h_flex().gap(S["space-1"]).items_center().text_size(S["type-caption"][0]).text_color(snoozed ? t["text-3"] : t["action-text"])
        .child(svg(`${ICONS}/${snoozed ? "clock" : "rotate-ccw"}.svg`).size(13).flex_none().text_color(snoozed ? t["text-3"] : t["action-text"])).child(row.note)
      : null;
    const head = table
      ? h_flex().gap(S["space-2"]).items_start().min_h(S["pane-row"] - 2 * S["space-2"])
        .child(h_flex().w(88).flex_none().gap(S["space-1"]).items_center().text_size(S["type-caption"][0]).text_color(reasonColor)
          .child(ui.tile(`word-${row.id}`, row.kind, "small")).child(row.kind))
        .child(div().w(96).flex_none().truncate().child(ui.mono(row.key ?? "")))
        .child(div().w(80).flex_none().truncate().text_size(S["type-caption"][0]).text_color(t["text-2"]).child(row.step ?? ""))
        .child(v_flex().flex_1().min_w_0().gap(S["space-1"])
          .child(div().whitespace_normal().text_size(S["type-small"][0]).text_color(row.kind === "ask" ? t.text : reasonColor).child(text))
          .when(Boolean(note), (el) => el.child(note))
          .when(Boolean(row.routed), (el) => el.child(routed(ui, row.routed)))
          .when(Boolean(row.runner), (el) => el.child(div().text_size(S["type-caption"][0]).text_color(row.away ? t["warn-text"] : t["text-2"]).child([row.runner, row.away].filter(Boolean).join(", "))))
          .when(Boolean(row.sent), (el) => el.child(div().text_size(S["type-caption"][0]).text_color(row.sent?.refused ? t["warn-text"] : t["action-text"]).child(row.sent?.text ?? "")))
          .when(Boolean(row.prs), (el) => el.child(div().text_size(S["type-caption"][0]).text_color(t["text-2"]).child(String(row.prs)))))
        .child(div().w(104).flex_none().truncate().text_size(S["type-caption"][0]).text_color(t["text-2"]).child(row.recommends ?? ""))
        .child(div().w(48).flex_none().text_right().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(row.count != null ? String(row.count) : row.age ?? ""))
      : h_flex().gap(S["space-3"]).items_start()
        .child(ui.tile(row.id, row.kind, "tile"))
        .child(v_flex().flex_1().min_w_0().gap(S["space-1"])
          .child(h_flex().gap(S["space-2"]).items_center().text_size(S["type-caption"][0])
            .child(ui.mono(row.key ?? ""))
            .child(div().truncate().text_color(t["text-2"]).child(row.step ?? ""))
            .when(row.kind !== "ask", (el) => el.child(div().text_color(reasonColor).child(row.kind)))
            .when(Boolean(row.runner), (el) => el.child(div().flex_none().px(S["space-1"]).rounded(S["radius-sm"]).bg(t.raised).font_weight(600).text_color(row.away ? t["warn-text"] : t["text-2"]).child(String(row.runner))))
            .when(Boolean(row.prs), (el) => el.child(div().flex_none().text_color(t["text-2"]).child(String(row.prs))))
            .child(div().flex_1())
            .child(div().flex_none().text_color(t["text-3"]).child(row.count != null ? String(row.count) : row.age ?? "")))
          .child(div().whitespace_normal().max_h(2 * S["type-body"][1]).overflow_hidden().text_size(S["type-body"][0])
            .text_color(row.kind === "ask" ? t.text : reasonColor).child(text))
          .when(Boolean(row.sent), (el) => el.child(div().text_size(S["type-caption"][0]).text_color(row.sent?.refused ? t["warn-text"] : t["action-text"]).child(row.sent?.text ?? "")))
          .when(Boolean(row.away) && !row.sent, (el) => el.child(div().text_size(S["type-caption"][0]).text_color(t["warn-text"]).child(`${row.runner} ${row.away}`)))
          .when(Boolean(meta) && !row.sent, (el) => el.child(div().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(String(meta))))
          .when(Boolean(note), (el) => el.child(note))
          .when(Boolean(row.routed), (el) => el.child(routed(ui, row.routed))));
    return v_flex().id(`row-${row.id}`).px(S["space-2"]).py(S["space-2"]).gap(S["space-2"]).min_h(S["pane-row"]).rounded(S["radius-md"])
      .border_t(1).border_color(selected ? t["action-soft"] : t.line)
      .bg(selected ? t["action-soft"] : t.bg).hover((s) => s.bg(selected ? t["action-soft"] : t.surface))
      .on_click((/** @type {unknown} */ _e, /** @type {import("gpui-kit").Context} */ cx) => {
        // The asleep summary row opens the Asleep view.
        if (row.id === "asleep") this.view = "asleep";
        else this.selected = row.id;
        this.focus.focus();
        cx.notify();
      })
      .child(head)
      .when(selected, (el) => el.child(this.detail(ui, row)));
  }

  // A selected row opens its detail in place, under the row, in the narrow list and the wide table.
  /** @param {Ui} ui @param {Row} row */
  detail(ui, row) {
    const { t } = ui;
    const item = row.snooze;
    const ask = row.ask;
    const restart = RESTART[row.kind];
    const actions = h_flex().gap(S["space-2"]).flex_wrap()
      .when(ask != null && !row.snoozed, (el) => el.child(ui.button(`answer-${row.id}`, "Answer…", () => this.answer(/** @type {number | string} */ (ask)), { primary: true })))
      .when(row.todo != null, (el) => el.child(ui.button(`done-${row.id}`, "Done", (_e, cx) => this.done(cx, /** @type {number} */ (row.todo)), { disabled: this.busy })))
      .when(row.report != null, (el) => el.child(ui.button(`read-${row.id}`, "Read", (_e, cx) => this.read(cx, /** @type {number} */ (row.report)), { disabled: this.busy })))
      .when(Boolean(restart) && row.task != null, (el) => el.child(ui.button(`restart-${row.id}`, restart, (_e, cx) => this.restart(cx, row), { disabled: this.busy })))
      .when(row.task != null, (el) => el.child(ui.button(`open-${row.id}`, "Open task", () => this.openTask(/** @type {number} */ (row.task))))
        .child(ui.button(`attach-${row.id}`, "Attach", () => this.attach(/** @type {number} */ (row.task)))))
      .when(Boolean(item), (el) => el
        .child(ui.button(`snooze-${row.id}`, row.snoozed ? "Unsnooze" : "Snooze 1 h",
          (_e, cx) => this.snooze(cx, /** @type {NonNullable<Row["snooze"]>} */ (item), row.snoozed ? null : HOUR), { icon: "clock", disabled: this.busy }))
        .child(ui.button(`tomorrow-${row.id}`, "Snooze until tomorrow 09:00",
          (_e, cx) => this.snooze(cx, /** @type {NonNullable<Row["snooze"]>} */ (item), { until: tomorrowAt(9, Date.now()) }), { icon: "moon", disabled: this.busy })));
    return v_flex().gap(S["space-2"]).pl(S["pane-tile"] + S["space-3"])
      .children((row.memories ?? []).map((/** @type {any} */ memory) => this.memory(ui, memory)))
      .children((row.progress ?? []).map((/** @type {string} */ line, /** @type {number} */ i) => div().whitespace_normal().text_size(S["type-caption"][0]).text_color(t["text-2"]).child(copyable(`progress-${row.id}-${i}`, String(line)))))
      .child(actions);
  }

  // One memory candidate: its name and tag, description, full text, the text it replaces on demand, then Accept and Drop.
  /** @param {Ui} ui @param {any} memory */
  memory(ui, memory) {
    const { t } = ui;
    const armed = this.memoryArmed?.id === memory.id;
    const sent = this.memorySent.get(memory.id);
    const shown = this.currentShown.has(memory.id);
    const text = (/** @type {string} */ id, /** @type {string} */ body) => div().whitespace_normal().px(S["space-3"]).py(S["space-2"]).rounded(S["radius-md"]).bg(t.raised).text_size(S["type-small"][0]).child(copyable(id, body));
    return v_flex().id(`memory-${memory.id}`).gap(S["space-2"]).pt(S["space-2"]).border_t(1).border_color(t.line)
      .child(h_flex().gap(S["space-2"]).items_center().text_size(S["type-caption"][0])
        .child(div().font_family(ui.fonts.mono).text_color(t.text).child(copyable(`memory-name-${memory.id}`, String(memory.name))))
        .child(div().px(S["space-1"]).rounded(S["radius-sm"]).border(1).border_color(memory.warn ? t["warn-text"] : t["control-line"]).text_color(memory.warn ? t["warn-text"] : t["text-2"]).child(String(memory.tag))))
      .when(Boolean(memory.description), (el) => el.child(div().whitespace_normal().text_size(S["type-caption"][0]).text_color(t["text-2"]).child(copyable(`memory-description-${memory.id}`, String(memory.description)))))
      .child(text(`memory-text-${memory.id}`, String(memory.text)))
      .when(memory.current != null, (el) => el
        .child(ui.button(`current-${memory.id}`, shown ? "Hide current text" : "Current text", (_e, cx) => {
          if (shown) this.currentShown.delete(memory.id);
          else this.currentShown.add(memory.id);
          cx.notify();
        }))
        .when(shown, (el) => el.child(text(`memory-current-${memory.id}`, String(memory.current)))))
      .child(div().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(String(memory.by)))
      .child(h_flex().gap(S["space-2"])
        .child(ui.button(`accept-${memory.id}`, armed ? "Press again to accept" : "Accept", (_e, cx) => this.decide(cx, memory.id, true), { primary: true, disabled: this.busy || Boolean(sent) }))
        .child(ui.button(`drop-${memory.id}`, "Drop", (_e, cx) => this.decide(cx, memory.id, false), { disabled: this.busy || Boolean(sent) })))
      .when(Boolean(sent), (el) => el.child(div().whitespace_normal().text_size(S["type-caption"][0]).text_color(t["action-text"]).child(String(sent))));
  }

  /** @param {Ui} ui @param {Model} model */
  upNext(ui, model) {
    const { t } = ui;
    return v_flex().gap(S["space-1"]).flex_none()
      .child(div().text_size(S["type-caption"][0]).font_weight(600).child("Up next"))
      .when(!model.upNext.length, (el) => el.child(ui.note(model.upNextEmpty)))
      // A line opens its task; a schedule that starts a new task names none, so its line opens nothing.
      .children(model.upNext.map((/** @type {Model["upNext"][number]} */ entry, /** @type {number} */ i) => h_flex().id(`up-next-${i}`).gap(S["space-2"]).py(S["space-1"]).items_start().text_size(S["type-caption"][0])
        .border_t(1).border_color(t.line)
        .when(entry.task != null, (el) => el.rounded(S["radius-sm"]).hover((s) => s.bg(t.surface)).on_click(() => this.openTask(entry.task)))
        .child(div().w(96).flex_none().text_color(t["text-2"]).child(entry.when))
        .child(div().flex_none().child(ui.mono(entry.key ?? "")))
        .child(div().flex_1().min_w_0().whitespace_normal().text_color(t["text-3"]).child(entry.why))));
  }

  /** @param {Ui} ui @param {Model} model */
  sinceYouLooked(ui, model) {
    const { t } = ui;
    return v_flex().gap(S["space-1"]).flex_none()
      .child(h_flex().gap(S["space-2"]).text_size(S["type-caption"][0])
        .child(div().font_weight(600).child("Since you looked"))
        .when(Boolean(model.since.from), (el) => el.child(div().text_color(t["text-3"]).child(String(model.since.from)))))
      .when(!model.since.rows.length, (el) => el.child(ui.note("Nothing happened since you looked. Steps that end, messages and reports show here.")))
      .children(model.since.rows.map((/** @type {Model["since"]["rows"][number]} */ event, /** @type {number} */ i) => h_flex().id(`since-${i}`).gap(S["space-2"]).py(S["space-1"]).items_start().text_size(S["type-caption"][0])
        .border_t(1).border_color(t.line)
        .child(div().flex_none().child(ui.mono(event.key ?? "")))
        .child(div().flex_1().min_w_0().whitespace_normal().text_color(t["text-2"]).child(copyable(`since-what-${i}`, event.what)))
        .child(div().flex_none().font_family(ui.fonts.mono).text_color(t["text-3"]).child(event.time))));
  }
}

// The Desk's small parts, drawn with Priory's tokens resolved for Herdr's appearance.
class Ui {
  /** @param {Record<string, import("gpui-kit").Color>} t @param {{ display: string, ui: string, mono: string }} fonts */
  constructor(t, fonts) {
    this.t = t;
    this.fonts = fonts;
  }

  /** @param {string} id @param {string} kind @param {"tile" | "small"} size */
  tile(id, kind, size) {
    const [role, icon] = TILES[kind] ?? MUTED_TILE;
    return stateTile(kind, icon && `${ICONS}/${icon}.svg`, role, this.t, size).id(`tile-${id}`);
  }

  /** @param {string} text */
  mono(text) {
    return div().font_family(this.fonts.mono).text_size(S["type-mono"][0]).text_color(this.t["text-2"]).child(text);
  }

  /** @param {string} text */
  note(text) {
    return div().whitespace_normal().text_size(S["type-small"][0]).text_color(this.t["text-3"]).child(text);
  }

  /** @param {string} id @param {string} text */
  banner(id, text) {
    return h_flex().id(id).gap(S["space-2"]).px(S["space-3"]).py(S["space-2"]).items_start().rounded(S["radius-md"]).bg(this.t.surface)
      .border(1).border_color(this.t["warn-text"]).text_size(S["type-small"][0])
      .child(svg(`${ICONS}/${/memory/.test(text) ? "memory-stick" : "hard-drive"}.svg`).size(16).flex_none().text_color(this.t["warn-text"]))
      .child(div().flex_1().min_w_0().whitespace_normal().child(copyable(`${id}-text`, text)));
  }

  /** @param {string[][]} hints @param {boolean} [inline] */
  hints(hints, inline = false) {
    const t = this.t;
    return h_flex().flex_none().gap(S["space-3"]).items_center().text_size(S["type-caption"][0]).text_color(t["text-3"])
      .when(!inline, (el) => el.px(S["space-4"]).py(S["space-2"]).border_t(1).border_color(t.line))
      .children(hints.map((hint) => h_flex().gap(S["space-1"]).items_center()
        .children(hint.slice(0, -1).map((key) => div().px(S["space-1"]).rounded(S["radius-sm"]).bg(t.raised).font_family(this.fonts.mono).child(key)))
        .child(hint[hint.length - 1])));
  }

  /**
   * @param {string} id @param {string} label
   * @param {(event: unknown, cx: import("gpui-kit").Context) => void} onClick
   * @param {{ primary?: boolean, icon?: string, disabled?: boolean }} [options]
   */
  button(id, label, onClick, { primary = false, icon, disabled = false } = {}) {
    const t = this.t;
    return Button.new(id).disabled(disabled).h(S["control-fine"]).px(S["space-3"]).gap(S["space-1"]).flex_none().items_center().rounded(S["radius-md"])
      .text_size(S["type-caption"][0]).font_weight(600).border(1)
      .border_color(primary ? t.action : t["control-line"]).bg(primary ? t.action : t.surface).text_color(primary ? t["on-action"] : t.text)
      .when(disabled, (el) => el.opacity(0.5))
      .when(!disabled, (el) => el.hover((s) => s.opacity(0.88)).active((s) => s.opacity(0.72)))
      .focus((s) => s.border_color(primary ? t["on-action"] : t.action))
      .on_mouse_down("left", (/** @type {unknown} */ _e, /** @type {import("gpui-kit").Context} */ cx) => cx.stop_propagation())
      .on_click(onClick)
      .when(Boolean(icon), (el) => el.child(svg(`${ICONS}/${icon}.svg`).size(14).text_color(primary ? t["on-action"] : t.text)))
      .child(label);
  }
}
