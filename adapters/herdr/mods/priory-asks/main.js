import { View, div, svg } from "gpui-kit";
import { h_flex, v_flex, Button, Popover, ScrollHandle, Textarea, TextareaState } from "gpui-base";
import { appHome, exec, home as herdrHome, readTextFile, setShared } from "herdr";
import { priory } from "./priory.js";
import { FONTS, palette } from "./tokens.js";
import { ARM_MS, askDetail, asksList, press, refusal, tomorrowAt } from "./view.js";

// The --pane-* tokens of design/tokens/structure.css, and the type and radii this panel uses.
const PANE = { split: 720, head: 44, chip: 28, row: 52, tile: 24, option: 40, bar: 52, control: 32 };
const RADIUS = { sm: 5, tile: 8, md: 12, card: 14 };
// Size and line height; gpui-kit reads a bare line height as a multiplier, so heights are lengths.
/** @type {Record<"question" | "body" | "small" | "caption", [number, `${number}px`]>} */
const TYPE = { question: [18, "24px"], body: [14, "20px"], small: [13, "18px"], caption: [12, "16px"] };
const UNDO_MS = 4000;
const RETRY_MS = 3000;
const ICON = (/** @type {string} */ name) => `mods/priory-asks/icons/${name}.svg`;

/** @typedef {Record<string, import("gpui-kit").Color>} Palette */
/** @typedef {typeof FONTS.indigo} Fonts */
/** @typedef {[string, string, (cx: import("gpui-kit").Context) => void, string?]} MenuItem a label, a key hint, an action and an icon */

// settings.json's "priory" object: { binary, machine, factory, theme }, each optional. Mirrors priory-tabs.
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

export default class PrioryAsks extends View {
  /** @param {unknown} _props @param {import("gpui-kit").AsyncContext} cx */
  init(_props, cx) {
    /** @type {{ theme: string, client: ReturnType<typeof priory> } | null} */
    this.priory = null;
    /** @type {{ asks: any[], revision: number } | null} */
    this.data = null;
    /** @type {string | null} */
    this.error = null;
    /** `priory task ID --json` of the selected ask's task: its brief, plan, pull request and runs. */
    /** @type {any} */
    this.task = null;
    this.updated = 0;
    this.width = 420;
    this.view = "open";
    /** @type {{ command?: boolean, lane?: string, band?: string, task?: string }} */
    this.facets = {};
    /** @type {string | number | null} */
    this.selected = null;
    /** The section the person reads below 720 px, with the answer bar under it; null shows the list. */
    /** @type {string | null} */
    this.reading = null;
    /** @type {import("./view.js").Armed | null} */
    this.armed = null;
    /** An answer waiting out its Undo, then in flight. */
    /** @type {{ id: string | number, text: string, timer: import("gpui-kit").Task | null } | null} */
    this.pending = null;
    /** Whether a snooze, dismiss or answer is in flight; nothing else starts until it ends. */
    this.acting = false;
    /** Why the last answer was refused, when its ask has since left the view. */
    /** @type {string | null} */
    this.flash = null;
    /** @type {Map<string | number, import("gpui-base").TextareaState>} */
    this.drafts = new Map();
    this.writing = new Set();
    /** @type {Map<string | number, string>} */
    this.notices = new Map();
    /** @type {string | null} which popover is open: "more", "options", "lane", "band" or "task" */
    this.popover = null;
    this.typing = false;
    this.focus = cx.focus_handle();
    /** @type {Fonts} */
    this.font = FONTS.indigo;
    this.scroll = ScrollHandle.new();
    this.refresh(cx);
  }

  /** Reads the asks, then waits for the next revision and reads again; keeps the last good list on failure. */
  /** @param {import("gpui-kit").AsyncContext} cx */
  refresh(cx) {
    cx.spawn(async (cx) => {
      for (;;) {
        try {
          this.priory ??= await connect();
          const read = await this.priory.client.asks();
          this.data = read;
          this.updated = Date.now();
          this.error = null;
          if (!this.rows().some((row) => row.id === this.selected)) this.selected = this.firstRow();
          await this.readTask();
          cx.notify();
          await this.priory.client.wait(read.revision);
        } catch (error) {
          this.priory = null;
          this.error = message(error);
          cx.notify();
          await cx.sleep(RETRY_MS);
        }
      }
    });
  }

  list(now = Date.now()) {
    return asksList(this.data?.asks ?? [], { view: this.view, now, bands: this.data?.bands, facets: this.facets });
  }

  rows() {
    return this.list().sections.flatMap((section) => section.rows);
  }

  /** Reads the selected ask's task; a task that cannot be read leaves its sections out. */
  async readTask() {
    const id = this.selectedAsk()?.task;
    const task = id == null || !this.priory ? null : await this.priory.client.task(id).catch(() => null);
    // A slower read for an earlier selection must not replace the current one's.
    if (this.selectedAsk()?.task === id) this.task = task;
  }

  firstRow() {
    return this.rows()[0]?.id ?? null;
  }

  busy() {
    return this.acting || this.pending !== null;
  }

  // Only a row the current view and facets show can be selected, so no key answers a hidden ask.
  selectedAsk() {
    if (!this.rows().some((row) => row.id === this.selected)) return null;
    return this.data?.asks.find((ask) => ask.id === this.selected) ?? null;
  }

  /** After a view or facet changes, the first row it shows opens. @param {import("gpui-kit").Context} cx */
  refilter(cx) {
    this.select(null, cx);
    this.selected = this.firstRow();
  }

  /** @param {string | number} id */
  draft(id) {
    let draft = this.drafts.get(id);
    if (draft) return draft;
    draft = TextareaState.new({ placeholder: "Your answer", rows: 3 });
    draft.on("focus", (_event, cx) => { this.typing = true; cx.notify(); });
    draft.on("blur", (_event, cx) => { this.typing = false; cx.notify(); });
    draft.on("change", (_event, cx) => cx.notify());
    this.drafts.set(id, draft);
    return draft;
  }

  /** @param {string | number | null} id @param {import("gpui-kit").Context} cx */
  select(id, cx) {
    this.focus.focus();
    this.selected = id;
    this.reading = null;
    this.popover = null;
    this.armed = null;
    this.task = null;
    cx.spawn(async (cx) => {
      await this.readTask();
      cx.notify();
    });
    cx.notify();
  }

  /** @param {number} step @param {import("gpui-kit").Context} cx */
  move(step, cx) {
    const ids = this.rows().map((row) => row.id);
    const at = ids.indexOf(/** @type {any} */ (this.selected));
    const next = ids[Math.min(ids.length - 1, Math.max(0, at + step))];
    if (next !== undefined) this.select(next, cx);
  }

  /** @param {any} ask @param {number} index @param {import("gpui-kit").Context} cx */
  choose(ask, index, cx) {
    if (this.busy() || this.error) return;
    const now = Date.now();
    const outcome = press(ask, index, this.armed, now);
    if ("arm" in outcome) {
      this.armed = outcome.arm;
      this.popover = null;
      this.drain(cx);
      cx.notify();
      return;
    }
    this.armed = null;
    this.popover = null;
    if (outcome.answer.run) return this.send(ask, outcome.answer.text, true, cx);
    this.hold(ask, outcome.answer.text, cx);
  }

  // Redraws the draining bar until the arming lapses, then shows the option unarmed.
  /** @param {import("gpui-kit").Context} cx */
  drain(cx) {
    cx.timer.after(100, (cx) => {
      if (!this.armed) return;
      if (Date.now() - this.armed.at >= ARM_MS) this.armed = null;
      else this.drain(cx);
      cx.notify();
    });
  }

  // Nothing leaves the panel until the Undo time passes, as DESIGN.md's sending state asks.
  /** @param {any} ask @param {string} text @param {import("gpui-kit").Context} cx */
  hold(ask, text, cx) {
    this.notices.delete(ask.id);
    const pending = { id: ask.id, text, timer: /** @type {import("gpui-kit").Task | null} */ (null) };
    pending.timer = cx.timer.after(UNDO_MS, (cx) => {
      pending.timer = null;
      this.send(ask, text, false, cx);
    });
    this.pending = pending;
    cx.notify();
  }

  /** @param {import("gpui-kit").Context} cx */
  undo(cx) {
    if (!this.pending?.timer) return;
    this.pending.timer.cancel();
    this.pending = null;
    cx.notify();
  }

  /** @param {any} ask @param {string} text @param {boolean} run @param {import("gpui-kit").Context} cx */
  send(ask, text, run, cx) {
    this.pending = { id: ask.id, text, timer: null };
    cx.notify();
    this.act(ask, (client) => client.answer(ask.id, text, ask.revision, { run }), cx, {
      sent: () => {
        const draft = this.drafts.get(ask.id);
        if (draft?.value().trim() === text) draft.set_value("");
        this.writing.delete(ask.id);
      },
      // The old answer stays in the text field, so a changed question can be answered again.
      refused: () => {
        if (!this.draft(ask.id).value().trim()) this.draft(ask.id).set_value(text);
        this.writing.add(ask.id);
      },
    });
  }

  /**
   * Runs one verb for `ask`. On success the next row opens; on failure the panel re-reads the asks and
   * says why on the ask.
   * @param {any} ask @param {(client: ReturnType<typeof priory>) => Promise<unknown>} verb @param {import("gpui-kit").Context} cx
   * @param {{ sent?: () => void, refused?: () => void }} [after] what an answer does with its draft
   */
  act(ask, verb, cx, after = {}) {
    this.acting = true;
    this.flash = null;
    cx.spawn(async (cx) => {
      const before = this.rows().map((row) => row.id);
      try {
        if (!this.priory) throw new Error("priory is not reachable");
        await verb(this.priory.client);
        after.sent?.();
        this.notices.delete(ask.id);
        this.data = await this.priory.client.asks();
        const left = this.rows().map((row) => row.id).filter((id) => id !== ask.id);
        this.selected = left.find((id) => before.indexOf(id) > before.indexOf(ask.id)) ?? left[0] ?? null;
        this.reading = null;
      } catch (error) {
        after.refused?.();
        /** @type {{ asks: any[], revision: number } | null} */
        let fresh = null;
        try {
          fresh = await this.priory?.client.asks() ?? null;
        } catch {
          fresh = null;
        }
        if (fresh) this.data = fresh;
        const notice = refusal(ask, fresh?.asks.find((/** @type {any} */ candidate) => candidate.id === ask.id), message(error), Date.now());
        if (this.rows().some((row) => row.id === ask.id)) this.notices.set(ask.id, notice);
        else {
          this.flash = `Ask ${ask.id}: ${notice}`;
          this.selected = this.firstRow();
          this.reading = null;
        }
      }
      this.acting = false;
      if (this.pending?.id === ask.id && !this.pending.timer) this.pending = null;
      cx.notify();
    });
  }

  /** @param {any} ask @param {{ for?: string, until?: string } | null} when @param {import("gpui-kit").Context} cx */
  snooze(ask, when, cx) {
    this.popover = null;
    if (this.busy()) return cx.notify();
    this.act(ask, (client) => client.snooze("ask", ask.id, when), cx);
  }

  /** @param {import("gpui-kit").KeyEvent} event @param {import("gpui-kit").Context} cx */
  key(event, cx) {
    const ask = this.selectedAsk();
    if (event.key === "escape") {
      if (this.popover) this.popover = null;
      else if (this.armed) this.armed = null;
      else if (ask && this.writing.delete(ask.id)) this.focus.focus();
      else this.reading = null;
      return cx.notify();
    }
    // A held key repeats only movement, so holding 1 never presses a command option twice.
    if (event.is_held && !["j", "k", "up", "down"].includes(event.key)) return;
    // Single keys never fire inside the answer field.
    if (this.typing || event.modifiers?.control || event.modifiers?.platform || event.modifiers?.alt) return;
    const detail = ask && askDetail(ask, { now: Date.now(), armed: this.armed, task: this.task });
    const digit = Number(event.key);
    if (event.key === "j" || event.key === "down") return this.move(1, cx);
    if (event.key === "k" || event.key === "up") return this.move(-1, cx);
    if (event.key === "enter" && ask && this.width < PANE.split) { this.reading = this.reading ? null : "why"; return cx.notify(); }
    if (!ask || !detail) return;
    if (digit >= 1 && digit <= detail.options.length) return this.choose(ask, digit - 1, cx);
    if (event.key === "w" && ask.state === "open") { this.writing.add(ask.id); this.draft(ask.id).focus(); return cx.notify(); }
    if (event.key === "s" && detail.canSnooze && !ask.snoozed_until) return this.snooze(ask, { for: "1h" }, cx);
    if (event.key === "[" || event.key === "]") {
      const chips = detail.chips.map(([id]) => id);
      const at = chips.indexOf(this.reading ?? "why");
      this.reading = chips[Math.min(chips.length - 1, Math.max(0, at + (event.key === "]" ? 1 : -1)))];
      return cx.notify();
    }
  }

  /** @param {import("gpui-kit").Context} cx */
  render(cx) {
    const theme = this.priory?.theme ?? "indigo";
    const t = /** @type {Palette} */ (palette(theme, cx.theme().appearance));
    this.font = FONTS[/** @type {keyof typeof FONTS} */ (theme)] ?? FONTS.indigo;
    const now = Date.now();
    const list = this.list(now);
    const ask = this.selectedAsk();
    const wide = this.width >= PANE.split;
    const open = list.views[0].count;
    const reading = !wide && ask && this.reading;
    return v_flex().id("priory-asks").size_full().min_h_0().bg(t.bg).text_color(t.text).font_family(this.font.ui).text_size(TYPE.body[0])
      .track_focus(this.focus).on_key_down((event, cx) => this.key(event, cx))
      .on_size_change(({ width }, cx) => {
        if (Math.abs(width - this.width) < 1) return;
        this.width = width;
        cx.notify();
      })
      .when(!reading, (el) => el.child(this.header(t, open)).child(this.filters(list, t)))
      .when(Boolean(this.error), (el) => el.child(this.offline(t, now)))
      .when(Boolean(this.flash) && !reading, (el) => el.child(h_flex().flex_none().px(16).py(6).gap(8).items_center().border_b(1).border_color(t.line)
        .text_size(TYPE.small[0]).text_color(t["warn-text"]).child(div().flex_1().whitespace_normal().child(this.flash ?? ""))
        .child(this.secondary("flash-close", "Close", (cx) => { this.flash = null; cx.notify(); }, t))))
      .child(reading ? this.reader(ask, t, now)
        : wide ? h_flex().flex_1().min_h_0()
          .child(this.rowsPane(list, t, false).w(Math.min(400, this.width * 0.42)).flex_none().border_r(1).border_color(t.line))
          .child(ask ? this.drawer(ask, t, now) : div().flex_1())
        : this.rowsPane(list, t, true).flex_1());
  }

  /** @param {Palette} t @param {number} open */
  header(t, open) {
    return h_flex().flex_none().h(PANE.head).px(16).gap(8).items_center().border_b(1).border_color(t.line)
      .child(div().font_family(this.font.display).font_weight(700).text_size(16).child("Asks"))
      .child(div().text_size(TYPE.small[0]).text_color(t["text-3"]).child(`${open} open`));
  }

  /** @param {ReturnType<typeof asksList>} list @param {Palette} t */
  filters(list, t) {
    /** @param {string} id @param {string} label @param {boolean} on @param {(cx: import("gpui-kit").Context) => void} action @param {number} radius */
    const chip = (id, label, on, action, radius) => Button.new(id).h(PANE.chip).px(10).flex().flex_none().items_center().gap(4).rounded(radius)
      .selected(on).text_size(TYPE.small[0]).font_weight(on ? 600 : 500)
      .bg(on ? t.emph : t.surface).text_color(on ? t["on-emph"] : t.text)
      .when(radius === RADIUS.sm, (el) => el.border(1).border_color(on ? t["action-soft"] : t["control-line"]))
      .hover((s) => s.opacity(0.88)).focus((s) => s.border(2).border_color(t["action-text"]))
      .on_click((_event, cx) => action(cx)).child(label);
    /** @param {"lane" | "band" | "task"} facet @param {string} label */
    const picker = (facet, label) => {
      const value = this.facets[facet];
      const trigger = Button.new(`facet-${facet}`).h(PANE.chip).px(10).flex().flex_none().items_center().gap(4).rounded(RADIUS.sm)
        .border(1).border_color(value ? t["action-soft"] : t["control-line"]).bg(value ? t["action-soft"] : t.surface)
        .text_color(value ? t["action-text"] : t.text).text_size(TYPE.small[0]).selected(this.popover === facet)
        .accessibility_label(value ? `${label}: ${value}. Clear` : `Filter by ${label.toLowerCase()}`)
        .focus((s) => s.border(2).border_color(t["action-text"]))
        .child(value ?? label).child(svg(ICON(value ? "x" : "chevron-down")).size(12).text_color(value ? t["action-text"] : t["text-3"]));
      if (value) return trigger.on_click((_event, cx) => { delete this.facets[facet]; this.refilter(cx); });
      return Popover.new(`facet-${facet}-menu`).trigger(trigger).anchor("top_left").open(this.popover === facet)
        .on_open_change((/** @type {boolean} */ next, /** @type {import("gpui-kit").Context} */ cx) => { this.popover = next ? facet : null; cx.notify(); })
        .content(this.menu(t, list.facets[facet].map(([name, count]) => [String(name), String(count), (cx) => {
          this.facets[facet] = String(name);
          this.refilter(cx);
        }])));
    };
    return v_flex().flex_none().px(16).py(8).gap(8).border_b(1).border_color(t.line)
      .child(h_flex().gap(6).flex_wrap().children(list.views.map((view) =>
        chip(`view-${view.id}`, `${view.label} ${view.count}`, this.view === view.id, (cx) => { this.view = view.id; this.refilter(cx); }, 999))))
      .child(h_flex().gap(6).flex_wrap()
        .child(chip("facet-command", "Command", Boolean(this.facets.command), (cx) => {
          if (this.facets.command) delete this.facets.command;
          else this.facets.command = true;
          this.refilter(cx);
        }, RADIUS.sm))
        .child(picker("lane", "Lane"))
        .child(picker("band", "Band"))
        .child(picker("task", "Task")));
  }

  /** @param {Palette} t @param {MenuItem[]} items */
  menu(t, items) {
    return v_flex().min_w(220).p(4).gap(2).rounded(RADIUS.md).bg(t.surface).border(1).border_color(t["control-line"])
      .children(items.map(([label, hint, action, icon], index) => Button.new(`menu-${index}-${label}`).h(PANE.control).px(8).gap(8).flex().items_center()
        .rounded(RADIUS.sm).text_size(TYPE.small[0]).text_color(t.text).hover((s) => s.bg(t.raised)).focus((s) => s.bg(t.raised))
        .on_click((_event, cx) => action(cx))
        .when(Boolean(icon), (el) => el.child(svg(ICON(/** @type {string} */ (icon))).size(14).text_color(t["text-2"])))
        .child(div().flex_1().child(label))
        .child(div().text_color(t["text-3"]).font_family(this.font.mono).text_size(TYPE.caption[0]).child(hint))));
  }

  /** @param {Palette} t @param {number} now */
  offline(t, now) {
    const minutes = Math.round((now - this.updated) / 60_000);
    const last = this.data ? ` Last update ${minutes < 1 ? "under a minute" : `${minutes} min`} ago.` : "";
    return h_flex().flex_none().px(16).py(6).gap(6).items_start().text_size(TYPE.small[0]).text_color(t["warn-text"]).border_b(1).border_color(t.line)
      .child(div().flex_1().whitespace_normal().child(`Reconnecting to priory: ${this.error}.${last}`));
  }

  /** @param {ReturnType<typeof asksList>} list @param {Palette} t @param {boolean} inPlace */
  rowsPane(list, t, inPlace) {
    const now = Date.now();
    const empty = {
      open: "No open asks. Agents ask here when they need you.",
      snoozed: "No snoozed asks. Snoozed asks leave the Open count; the agent keeps waiting.",
      answered: "No answered asks yet. An ask moves here once someone answers it.",
      all: "No asks yet. Agents ask here when they need you.",
    }[this.view] ?? "";
    if (!this.data) return v_flex().id("asks-list").p(16).text_color(t["text-3"]).child(this.error ? "" : "Reading the asks…");
    return v_flex().id("asks-list").min_h_0().overflow_y_scrollbar().pb(16)
      .when(this.view === "snoozed" && list.sections.length > 0, (el) => el.child(div().px(16).pt(10).text_size(TYPE.caption[0]).text_color(t["text-3"])
        .child("Snoozed asks leave the Open count. The agent keeps waiting, and a new revision brings an ask back early.")))
      .when(!list.sections.length, (el) => el.child(div().p(16).text_color(t["text-2"]).child(empty)))
      .children(list.sections.map((section) => v_flex()
        .child(h_flex().px(16).pt(14).pb(6).gap(6).text_size(TYPE.caption[0]).font_weight(600).text_color(t["text-2"])
          .child(section.title).child(div().font_weight(400).text_color(t["text-3"]).child(section.note)))
        .children(section.rows.map((row) => {
          const ask = this.data?.asks.find((candidate) => candidate.id === row.id);
          const selected = row.id === this.selected;
          return v_flex().px(8)
            .child(this.row(row, selected, t))
            .when(inPlace && selected && Boolean(ask), (el) => el.child(v_flex().px(8).pb(12).gap(12)
              .child(this.decide(ask, t, now)).child(this.chips(ask, t)).child(this.section(ask, "why", t, now))));
        }))));
  }

  /** @param {ReturnType<typeof asksList>["sections"][number]["rows"][number]} row @param {boolean} selected @param {Palette} t */
  row(row, selected, t) {
    return Button.new(`row-${row.id}`).w_full().min_h(PANE.row).px(8).py(8).flex().items_start().gap(10).rounded(RADIUS.md)
      .selected(selected).bg(selected ? t["action-soft"] : t.bg).hover((s) => s.bg(selected ? t["action-soft"] : t.raised))
      .focus((s) => s.border(2).border_color(t["action-text"]))
      .accessibility_label(`${row.tile === "ask" ? "Open ask" : row.tile} from ${row.key}: ${row.text}`)
      .on_click((_event, cx) => this.select(selected && this.width < PANE.split ? null : row.id, cx))
      .child(this.tile(row.tile, t))
      .child(v_flex().flex_1().min_w_0().gap(2)
        .child(h_flex().gap(6).items_center().text_size(TYPE.caption[0]).text_color(t["text-3"])
          .child(div().font_family(this.font.mono).text_color(t["text-2"]).child(row.key ?? ""))
          .child(div().child(row.step ?? ""))
          .when(row.command, (el) => el.child(this.tag("Command", t)))
          .child(div().flex_1())
          .child(div().whitespace_nowrap().child(row.age)))
        .child(div().whitespace_normal().text_size(TYPE.body[0]).line_height(TYPE.body[1]).font_weight(selected ? 600 : 500).child(row.text))
        .when(Boolean(row.meta), (el) => el.child(h_flex().gap(6).text_size(TYPE.caption[0]).text_color(t["text-3"])
          .child(row.meta?.[0] ?? "").child(div().min_w_0().truncate().font_weight(600).text_color(t["text-2"]).child(row.meta?.[1] ?? ""))))
        .when(Boolean(row.snoozed), (el) => el.child(this.note("clock", row.snoozed ?? "", t["text-3"])))
        .when(Boolean(row.backEarly), (el) => el.child(this.note("rotate-ccw", row.backEarly ?? "", t["action-text"]))));
  }

  /** @param {string} kind ask, answered or withdrawn @param {Palette} t */
  tile(kind, t) {
    const [fill, icon, ink] = kind === "ask" ? [t.wait, "ask", t["on-wait"]]
      : kind === "refused" ? [t["tile-warn"], "x", t["on-tile"]]
      : [t["tile-muted"], kind === "answered" ? "check" : "minus", t["on-tile"]];
    return h_flex().size(PANE.tile).flex_none().items_center().justify_center().rounded(RADIUS.tile).bg(fill)
      .accessibility_label(kind === "ask" ? "ask" : kind).child(svg(ICON(icon)).size(14).text_color(ink));
  }

  /** @param {string} label @param {Palette} t */
  tag(label, t) {
    return div().px(5).rounded(RADIUS.sm).bg(t.raised).text_size(TYPE.caption[0] - 1).font_weight(600).text_color(t["text-2"]).child(label);
  }

  /** @param {string} icon @param {string} text @param {import("gpui-kit").Color} color */
  note(icon, text, color) {
    return h_flex().gap(4).items_center().text_size(TYPE.caption[0]).text_color(color).child(svg(ICON(icon)).size(12).flex_none()).child(text);
  }

  /** The decide part: context, question, recommendation, options, write an answer and the overflow menu. */
  /** @param {any} ask @param {Palette} t @param {number} now */
  decide(ask, t, now) {
    const detail = askDetail(ask, { now, armed: this.armed, task: this.task });
    const notice = this.notices.get(ask.id);
    return v_flex().gap(10)
      .child(h_flex().flex_wrap().gap(6).items_center().text_size(TYPE.caption[0]).text_color(t["text-3"])
        .child(this.tile(ask.state === "open" ? "ask" : ask.state, t))
        .children(detail.context.map((word, index) => word === "Command" ? this.tag(word, t)
          : div().when(index === 0, (el) => el.font_family(this.font.mono).text_color(t["text-2"])).child(word))))
      .child(div().whitespace_normal().font_family(this.font.display).font_weight(700).text_size(TYPE.question[0]).line_height(TYPE.question[1]).child(detail.question))
      .when(Boolean(notice), (el) => el.child(div().whitespace_normal().text_size(TYPE.small[0]).text_color(t["warn-text"]).child(notice ?? "")))
      .when(Boolean(detail.decided), (el) => el.child(div().p(12).rounded(RADIUS.md).bg(t.raised).font_weight(600).child(detail.decided ?? "")))
      .when(Boolean(detail.ran), (el) => el.child(this.note("terminal", detail.ran?.text ?? "", detail.ran?.warn ? t["warn-text"] : t["text-2"])))
      .when(Boolean(detail.recommendation) && ask.state === "open", (el) => el.child(v_flex().p(12).gap(6).rounded(RADIUS.md).bg(t["action-soft"])
        .child(div().whitespace_normal().font_weight(600).text_color(t["action-text"]).child(detail.recommendation?.text ?? ""))
        .when(Boolean(detail.recommendation?.next), (el) => el.child(h_flex().gap(6).items_start().text_size(TYPE.small[0]).text_color(t["text-2"])
          .child(svg(ICON("corner-down-right")).size(14).flex_none().mt(2)).child(div().whitespace_normal().child(detail.recommendation?.next ?? ""))))))
      .when(detail.options.length > 0, (el) => el.child(this.options(ask, detail, t, now)))
      .when(Boolean(this.error) && ask.state === "open", (el) => el.child(this.note("x", "Reconnect to answer.", t["text-2"])))
      .when(Boolean(this.pending && this.pending.id === ask.id), (el) => el.child(this.sending(t)))
      .when(ask.state === "open", (el) => el.child(this.actions(ask, detail, t)))
      .when(ask.state === "open" && this.writing.has(ask.id), (el) => el.child(this.composer(ask, t)));
  }

  /** @param {any} ask @param {ReturnType<typeof askDetail>} detail @param {Palette} t @param {number} now */
  options(ask, detail, t, now) {
    const busy = this.busy() || Boolean(this.error);
    return v_flex().gap(6).children(detail.options.flatMap((option) => {
      const chosen = this.pending?.id === ask.id && this.pending.text === option.label;
      const fill = option.armed ? t.emph : option.recommended ? t.action : t.surface;
      const ink = option.armed ? t["on-emph"] : option.recommended ? t["on-action"] : t.text;
      const left = this.armed ? Math.max(0, 1 - (now - this.armed.at) / ARM_MS) : 0;
      const button = Button.new(`option-${ask.id}-${option.index}`).w_full().min_h(PANE.option).px(12).py(6).flex().items_center().gap(8)
        .rounded(RADIUS.card).bg(fill).text_color(ink).disabled(busy && !chosen)
        .when(!option.recommended && !option.armed, (el) => el.border(1).border_color(t["control-line"]))
        .when(option.armed, (el) => el.border(3).border_color(t.action))
        .opacity(option.dimmed || (busy && !chosen) ? 0.45 : 1)
        .focus((s) => s.border(2).border_color(t["action-text"]))
        .accessibility_label(option.armed ? `Press ${option.key} again or click to run ${option.label}` : [option.label, option.notes].filter(Boolean).join(". "))
        .on_click((_event, cx) => this.choose(ask, option.index, cx))
        .when(chosen, (el) => el.child(svg(ICON("check")).size(16).flex_none()))
        .child(v_flex().flex_1().min_w_0().gap(2)
          .when(option.armed, (el) => el.child(div().text_size(TYPE.caption[0]).font_weight(600).child(`Press ${option.key} again or click to run`)))
          .child(div().whitespace_normal().text_size(TYPE.body[0]).font_weight(600).child(option.label))
          .when(option.armed, (el) => el.child(div().text_size(TYPE.caption[0]).opacity(0.8).child(`Cancels in ${Math.ceil(left * ARM_MS / 1000)} s`))
            .child(div().h(3).w_full().rounded(2).bg(t["on-emph-2"]).child(div().h(3).w(`${Math.round(left * 100)}%`).rounded(2).bg(t.action))))
          .when(!option.armed && Boolean(option.notes), (el) => el.child(div().text_size(TYPE.caption[0]).opacity(0.85).child(option.notes))))
        .when(Boolean(option.key), (el) => el.child(div().px(5).rounded(RADIUS.sm).border(1).border_color(ink).opacity(0.7)
          .font_family(this.font.mono).text_size(TYPE.caption[0]).child(option.key)));
      return option.armed && detail.command ? [button, this.commandBlock(detail.command, t)] : [button];
    }));
  }

  /** @param {Palette} t */
  sending(t) {
    const pending = /** @type {NonNullable<typeof this.pending>} */ (this.pending);
    return h_flex().gap(8).p(10).items_center().rounded(RADIUS.md).bg(t.raised).text_size(TYPE.small[0])
      .child(div().flex_1().whitespace_normal().child(`Sending "${pending.text}"`))
      .when(Boolean(pending.timer), (el) => el.child(this.secondary("undo", "Undo", (cx) => this.undo(cx), t)));
  }

  /** @param {string} id @param {string} label @param {(cx: import("gpui-kit").Context) => void} action @param {Palette} t */
  secondary(id, label, action, t) {
    return Button.new(id).h(PANE.control).px(12).flex().flex_none().items_center().rounded(RADIUS.card).border(1).border_color(t["control-line"])
      .bg(t.surface).text_color(t.text).text_size(TYPE.small[0]).font_weight(600).hover((s) => s.bg(t.raised))
      .focus((s) => s.border(2).border_color(t["action-text"])).on_click((_event, cx) => action(cx)).child(label);
  }

  /** @param {string} id @param {string} icon @param {string} label @param {(cx: import("gpui-kit").Context) => void} action @param {Palette} t */
  ghost(id, icon, label, action, t) {
    return Button.new(id).h(PANE.control).px(6).flex().flex_none().items_center().gap(4).rounded(RADIUS.sm)
      .text_color(t["action-text"]).text_size(TYPE.small[0]).font_weight(500).hover((s) => s.bg(t.raised))
      .disabled(this.busy()).focus((s) => s.bg(t.raised)).on_click((_event, cx) => action(cx))
      .child(svg(ICON(icon)).size(14)).child(label);
  }

  /** "Write an answer", the snooze beside it, and the overflow menu. */
  /** @param {any} ask @param {ReturnType<typeof askDetail>} detail @param {Palette} t */
  actions(ask, detail, t) {
    const snoozed = Boolean(ask.snoozed_until) && !ask.back_early;
    return h_flex().gap(4).items_center()
      .child(this.ghost(`write-${ask.id}`, "pencil", "Write an answer…", (cx) => {
        this.writing.add(ask.id);
        this.draft(ask.id).focus();
        cx.notify();
      }, t))
      .child(div().flex_1())
      .when(detail.canSnooze, (el) => el.child(snoozed
        ? this.ghost(`unsnooze-${ask.id}`, "clock", "Unsnooze", (cx) => this.snooze(ask, null, cx), t)
        : this.ghost(`snooze-${ask.id}`, "clock", "Snooze 1 h", (cx) => this.snooze(ask, { for: "1h" }, cx), t)))
      .child(this.more(ask, detail, t));
  }

  /** @param {any} ask @param {ReturnType<typeof askDetail>} detail @param {Palette} t */
  more(ask, detail, t) {
    const open = this.popover === "more";
    const trigger = Button.new(`more-${ask.id}`).size(PANE.control).flex().flex_none().items_center().justify_center().rounded(RADIUS.sm)
      .selected(open).bg(open ? t.raised : t.bg).hover((s) => s.bg(t.raised)).focus((s) => s.bg(t.raised))
      .accessibility_label("More for this ask").child(svg(ICON("more-horizontal")).size(16).text_color(t["text-2"]));
    const snoozed = Boolean(ask.snoozed_until) && !ask.back_early;
    /** @type {MenuItem[]} */
    const items = [];
    if (detail.canSnooze && snoozed) items.push(["Unsnooze", "", (cx) => this.snooze(ask, null, cx), "clock"]);
    if (detail.canSnooze && !snoozed) {
      items.push(["Snooze 1 h", "S", (cx) => this.snooze(ask, { for: "1h" }, cx), "clock"]);
      items.push(["Snooze until tomorrow 09:00", "", (cx) => this.snooze(ask, { until: tomorrowAt(9, Date.now()) }, cx), "clock"]);
    }
    if (ask.task != null) {
      items.push(["Attach to task", "", (cx) => {
        this.popover = null;
        setShared("priory.attach", { task: ask.task, at: Date.now() });
        cx.notify();
      }, "terminal"]);
    }
    items.push(["Dismiss", "", (cx) => {
      this.popover = null;
      if (!this.busy()) this.act(ask, (client) => client.dismiss(ask.id, ask.revision), cx);
      cx.notify();
    }, "x"]);
    return Popover.new(`more-menu-${ask.id}`).trigger(trigger).anchor("bottom_right").open(open)
      .on_open_change((/** @type {boolean} */ next, /** @type {import("gpui-kit").Context} */ cx) => { this.popover = next ? "more" : null; cx.notify(); })
      .content(this.menu(t, items));
  }

  /** @param {any} ask @param {Palette} t */
  composer(ask, t) {
    const draft = this.draft(ask.id);
    const text = draft.value().trim();
    const send = (/** @type {import("gpui-kit").Context} */ cx) => {
      if (text && !this.busy()) this.hold(ask, text, cx);
    };
    return v_flex().gap(6)
      .child(div().id(`draft-${ask.id}`).p(8).rounded(RADIUS.md).border(1).border_color(t["control-line"]).bg(t.surface).cursor_text()
        .on_key_down((event, cx) => {
          if (event.keystroke !== "ctrl-enter" && event.keystroke !== "cmd-enter") return;
          send(cx);
          cx.stop_propagation();
        })
        .child(Textarea.new(draft).w_full().text_size(16).text_color(t.text)))
      .child(h_flex().gap(8).items_center()
        .child(div().flex_1().text_size(TYPE.caption[0]).text_color(t["text-3"]).child("Ctrl+Enter sends. Free text goes as the answer."))
        .child(Button.new(`send-${ask.id}`).h(PANE.control).px(14).flex().items_center().rounded(RADIUS.card).bg(t.action).text_color(t["on-action"])
          .font_weight(600).disabled(!text || this.busy()).opacity(!text || this.busy() ? 0.5 : 1)
          .focus((s) => s.border(2).border_color(t["action-text"])).on_click((_event, cx) => send(cx)).child("Send")));
  }

  /** @param {any} ask @param {Palette} t */
  chips(ask, t) {
    const detail = askDetail(ask, { now: Date.now(), armed: null, task: this.task });
    const current = this.reading ?? (this.width >= PANE.split ? "why" : null);
    return h_flex().flex_none().gap(4).flex_wrap().children(detail.chips.map(([id, label]) =>
      Button.new(`chip-${ask.id}-${id}`).h(PANE.chip).px(10).flex().items_center().rounded(999).selected(current === id)
        .bg(current === id ? t.emph : t.bg).text_color(current === id ? t["on-emph"] : t["text-2"]).text_size(TYPE.small[0]).font_weight(500)
        .hover((s) => s.bg(current === id ? t.emph : t.raised)).focus((s) => s.border(2).border_color(t["action-text"]))
        .on_click((_event, cx) => {
          this.reading = id;
          if (this.width >= PANE.split) this.scroll.scroll_to_item(detail.chips.findIndex(([chip]) => chip === id));
          cx.notify();
        })
        .child(label)));
  }

  /** @param {any} ask @param {string} id @param {Palette} t @param {number} now */
  section(ask, id, t, now) {
    const detail = askDetail(ask, { now, armed: this.armed, task: this.task });
    /** @param {string} icon @param {string} title @param {string} [meta] */
    const head = (icon, title, meta = "") => h_flex().gap(6).items_center().text_size(TYPE.small[0]).font_weight(650)
      .child(svg(ICON(icon)).size(14).text_color(t["text-2"])).child(title).child(div().flex_1())
      .child(div().font_weight(400).text_color(t["text-3"]).text_size(TYPE.caption[0]).child(meta));
    /** @param {string} icon @param {string} label @param {string} text */
    const fact = (icon, label, text) => h_flex().gap(6).items_start().text_size(TYPE.small[0]).text_color(t["text-2"])
      .child(svg(ICON(icon)).size(14).flex_none().mt(3)).child(div().flex_1().whitespace_normal().child(`${label} ${text}`));
    if (id === "history") {
      return v_flex().gap(8).child(head("history", "Answer history"))
        .children(detail.history.map((item) => h_flex().gap(8).items_center().text_size(TYPE.small[0])
          .child(this.tile(item.tile, t)).child(div().flex_1().whitespace_normal()
            .text_color(item.tile === "refused" ? t["warn-text"] : t.text).child(item.text))
          .child(div().font_family(this.font.mono).text_size(TYPE.caption[0]).text_color(t["text-3"]).child(item.time))));
    }
    /** @param {string} text */
    const body = (text) => div().whitespace_normal().text_size(TYPE.body[0]).line_height("22px").child(text);
    /** @param {string} text */
    const caption = (text) => div().whitespace_normal().text_size(TYPE.caption[0]).text_color(t["text-3"]).child(text);
    // Briefs, plans and results are agent text: they draw as plain text, never as Markdown or HTML.
    if (id === "brief" && detail.brief) {
      return v_flex().gap(8).child(head("message-text", "Brief", detail.brief.from ?? ""))
        .when(Boolean(detail.brief.request), (el) => el.child(caption("The request")).child(body(detail.brief.request)))
        .when(Boolean(detail.brief.text), (el) => el.child(body(detail.brief.text)));
    }
    if (id === "plan" && detail.plan) {
      return v_flex().gap(8).child(head("check", "Plan", detail.plan.from)).child(body(detail.plan.text));
    }
    if (id === "pr" && detail.pr) {
      return v_flex().gap(8).child(head("terminal", detail.pr.title, detail.pr.review ?? ""))
        .child(caption(detail.pr.line))
        .children(detail.pr.checks.map((check) => h_flex().gap(6).items_center().text_size(TYPE.small[0])
          .child(svg(ICON(check.ok ? "check" : "x")).size(14).text_color(check.ok ? t["ok-text"] : t["danger-text"]))
          .child(div().flex_1().child(check.name)).child(div().text_color(t["text-3"]).child(check.text))));
    }
    if (id === "trace") {
      return v_flex().gap(8).child(head("clock", "Trace"))
        .children(detail.trace.map((run) => h_flex().gap(8).items_center().text_size(TYPE.small[0])
          .child(div().w(138).flex_none().font_family(this.font.mono).child(run.label))
          .child(div().flex_1().whitespace_normal().text_color(t["text-2"]).child([run.span, run.outcome, run.tokens].filter(Boolean).join(", ")))));
    }
    if (id === "command" && detail.command) {
      return v_flex().gap(8).child(head("terminal", detail.command.title)).child(this.commandBlock(detail.command, t));
    }
    // Agent text is untrusted, so it draws as plain text, never as Markdown or HTML.
    return v_flex().gap(8)
      .child(head("message-text", "What the agent says", detail.why.run ? `run ${detail.why.run}` : ""))
      .child(div().whitespace_normal().text_size(TYPE.body[0]).line_height("22px").child(detail.why.text))
      .when(Boolean(detail.why.tradeoff), (el) => el.child(fact("scale", "Trade-off.", detail.why.tradeoff)))
      .when(Boolean(detail.why.whatWaits), (el) => el.child(fact("hourglass", "What waits.", detail.why.whatWaits)));
  }

  /** The argv word by word, each in its own box, so a space, a quote or an empty word shows as it is. */
  /** @param {NonNullable<ReturnType<typeof askDetail>["command"]>} command @param {Palette} t */
  commandBlock(command, t) {
    return v_flex().gap(6).p(10).rounded(RADIUS.md).bg(t.raised)
      .child(h_flex().flex_wrap().gap(4).children(command.words.map((word) => div().min_w(14).px(4).rounded(RADIUS.sm).border(1)
        .border_color(t["control-line"]).bg(t.surface).font_family(this.font.mono).text_size(12.5).whitespace_nowrap().child(word))))
      .child(div().whitespace_normal().text_size(TYPE.caption[0]).text_color(t["text-2"]).child(command.note));
  }

  /** From 720 px: the decide part pinned at the top, and the sections scrolling under it. */
  /** @param {any} ask @param {Palette} t @param {number} now */
  drawer(ask, t, now) {
    const detail = askDetail(ask, { now, armed: this.armed, task: this.task });
    return v_flex().flex_1().min_w_0().min_h_0()
      .child(v_flex().id("ask-decide").flex_none().max_h("50%").overflow_y_scrollbar().p(16).gap(12).border_b(1).border_color(t.line)
        .child(this.decide(ask, t, now)).child(this.chips(ask, t)))
      .child(v_flex().id("ask-sections").flex_1().min_h_0().overflow_y_scrollbar().p(16).gap(20)
        .track_scroll(this.scroll)
        .children(detail.chips.map(([id]) => this.section(ask, id, t, now))));
  }

  /** Below 720 px, reading a section: a one-line header, the chips, the section, and the answer bar. */
  /** @param {any} ask @param {Palette} t @param {number} now */
  reader(ask, t, now) {
    const detail = askDetail(ask, { now, armed: this.armed, task: this.task });
    const open = this.rows().filter((row) => row.tile === "ask").map((row) => row.id);
    const at = open.indexOf(ask.id);
    /** @param {string} id @param {string} icon @param {string} label @param {(cx: import("gpui-kit").Context) => void} action @param {boolean} [off] */
    const icon = (id, icon, label, action, off = false) => Button.new(id).size(PANE.control).flex().flex_none().items_center().justify_center()
      .rounded(RADIUS.sm).disabled(off).opacity(off ? 0.4 : 1).accessibility_label(label).hover((s) => s.bg(t.raised)).focus((s) => s.bg(t.raised))
      .on_click((_event, cx) => action(cx)).child(svg(ICON(icon)).size(16).text_color(t["text-2"]));
    return v_flex().flex_1().min_h_0()
      .child(h_flex().flex_none().h(PANE.head).px(8).gap(4).items_center().border_b(1).border_color(t.line)
        .child(icon("back", "chevron-left", "Back to the asks", (cx) => { this.reading = null; cx.notify(); }))
        .child(v_flex().flex_1().min_w_0()
          .child(div().truncate().font_weight(600).text_size(TYPE.small[0]).child(detail.question))
          .child(div().truncate().text_size(TYPE.caption[0]).text_color(t["text-3"]).child(detail.context.filter((word) => word !== "Command").slice(0, 2).concat(`revision ${ask.revision}`).join(" "))))
        .when(at >= 0, (el) => el.child(div().font_family(this.font.display).font_weight(700).text_size(TYPE.small[0]).child(`${at + 1}/${open.length}`)))
        .child(icon("previous", "chevron-up", "Previous ask", (cx) => { this.select(open[at - 1], cx); this.reading = "why"; }, at <= 0))
        .child(icon("next", "chevron-down", "Next ask", (cx) => { this.select(open[at + 1], cx); this.reading = "why"; }, at < 0 || at >= open.length - 1)))
      .child(div().flex_none().px(16).py(8).border_b(1).border_color(t.line).child(this.chips(ask, t)))
      .child(v_flex().id("ask-reading").flex_1().min_h_0().overflow_y_scrollbar().p(16).gap(12)
        .when(this.notices.has(ask.id), (el) => el.child(div().whitespace_normal().text_size(TYPE.small[0]).text_color(t["warn-text"]).child(this.notices.get(ask.id) ?? "")))
        .when(Boolean(detail.decided), (el) => el.child(div().p(12).rounded(RADIUS.md).bg(t.raised).font_weight(600).child(detail.decided ?? "")))
        .child(this.section(ask, this.reading ?? "why", t, now))
        .when(ask.state === "open" && this.writing.has(ask.id), (el) => el.child(this.composer(ask, t))))
      .when(Boolean(detail.bar), (el) => el.child(this.bar(ask, detail, t, now)));
  }

  /** The sticky answer bar: "3 options", the recommended option, and the overflow menu. */
  /** @param {any} ask @param {ReturnType<typeof askDetail>} detail @param {Palette} t @param {number} now */
  bar(ask, detail, t, now) {
    const bar = /** @type {NonNullable<typeof detail.bar>} */ (detail.bar);
    // An armed option takes the bar, so its command and second press stay in sight.
    const option = detail.options.find((candidate) => candidate.armed) ?? detail.options[bar.option];
    const opened = this.popover === "options";
    const trigger = Button.new(`options-${ask.id}`).h(PANE.control).px(10).flex().flex_none().items_center().gap(4).rounded(RADIUS.card)
      .border(1).border_color(t["control-line"]).bg(opened ? t.raised : t.surface).selected(opened).text_size(TYPE.small[0]).font_weight(600)
      .focus((s) => s.border(2).border_color(t["action-text"])).child(svg(ICON("chevron-up")).size(14)).child(bar.count);
    return v_flex().flex_none().gap(6).px(12).py(10).bg(t.bar).border_t(1).border_color(t.line)
      .when(option.armed && Boolean(detail.command), (el) => el.child(this.commandBlock(/** @type {any} */ (detail.command), t)))
      .when(Boolean(this.pending && this.pending.id === ask.id), (el) => el.child(this.sending(t)))
      .child(h_flex().min_h(PANE.bar - 20).gap(8).items_center()
        .child(Popover.new(`options-menu-${ask.id}`).trigger(trigger).anchor("bottom_left").open(opened)
          .on_open_change((/** @type {boolean} */ next, /** @type {import("gpui-kit").Context} */ cx) => { this.popover = next ? "options" : null; cx.notify(); })
          .content(v_flex().w(Math.max(260, this.width - 40)).p(10).gap(8).rounded(RADIUS.md).bg(t.surface).border(1).border_color(t["control-line"])
            .child(div().text_size(TYPE.small[0]).font_weight(600).child(`Answer ask #${ask.id}, revision ${ask.revision}`))
            .child(this.options(ask, detail, t, now))))
        .child(Button.new(`bar-answer-${ask.id}`).flex_1().min_w_0().min_h(PANE.control).px(12).py(4).flex().items_center().rounded(RADIUS.card)
          .bg(option.armed ? t.emph : t.action).text_color(option.armed ? t["on-emph"] : t["on-action"])
          .when(option.armed, (el) => el.border(3).border_color(t.action)).disabled(this.busy() || Boolean(this.error))
          .focus((s) => s.border(2).border_color(t["action-text"])).on_click((_event, cx) => this.choose(ask, option.index, cx))
          .child(v_flex().min_w_0()
            .child(div().truncate().font_weight(600).child(option.armed ? `Press ${option.key} again or click to run ${option.label}` : option.label))
            .child(div().truncate().text_size(TYPE.caption[0]).opacity(0.85)
              .child(option.armed ? `Cancels in ${Math.ceil(Math.max(0, ARM_MS - (now - (this.armed?.at ?? now))) / 1000)} s` : option.notes || "First option"))))
        .child(this.more(ask, detail, t)));
  }
}
