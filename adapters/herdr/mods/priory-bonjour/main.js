import { View, div, svg } from "gpui-kit";
import { h_flex, v_flex, Button, ScrollHandle } from "gpui-base";
import { appHome, exec, home as herdrHome, readTextFile } from "herdr";
import { priory } from "./priory.js";
import { FONTS, palette } from "./tokens.js";
import { dossier, nextPick, refusal } from "./view.js";

// The --pane-* tokens of design/tokens/structure.css, and the type and radii this panel uses.
const PANE = { head: 44, chip: 28, row: 52, tile: 24, control: 32 };
const RADIUS = { sm: 5, tile: 8, md: 12, card: 14 };
// Size and line height; gpui-kit reads a bare line height as a multiplier, so heights are lengths.
/** @type {Record<"body" | "small" | "caption", [number, `${number}px`]>} */
const TYPE = { body: [14, "20px"], small: [13, "18px"], caption: [12, "16px"] };
// From this width the sections list sits on the left (DESIGN.md, Bonjour).
const RAIL = { from: 1024, width: 200 };
const UNDO_MS = 4000;
const RETRY_MS = 3000;
const ICON = (/** @type {string} */ name) => `mods/priory-bonjour/icons/${name}.svg`;
// A task state's tile: its fill, its ink and its icon, as DESIGN.md's state tiles; null draws a dot.
/** @type {Record<string, [string, string, string | null]>} */
const TILES = {
  queued: ["tile-muted", "on-tile", "circle-dashed"], held: ["tile-hold", "on-tile", "pause"], ready: ["tile-live", "on-tile-live", "circle-dot"],
  running: ["tile-live", "on-tile-live", null], asleep: ["tile-muted", "on-tile", "moon"], blocked: ["tile-warn", "on-tile", "hand"],
  done: ["tile-ok", "on-tile", "check"], failed: ["tile-danger", "on-tile", "circle-x"], dropped: ["tile-muted", "on-tile", "minus"],
  moved: ["tile-muted", "on-tile", "arrow-right"], lost: ["tile-warn", "on-tile", "unplug"], ask: ["wait", "on-wait", "ask"],
  answered: ["tile-muted", "on-tile", "check"], withdrawn: ["tile-muted", "on-tile", "minus"],
};
// `priory report show`'s error before any task wrote a report: an empty state, not a failure.
const NO_REPORT = "no task has a report";

/** @typedef {Record<string, import("gpui-kit").Color>} Palette */
/** @typedef {typeof FONTS.indigo} Fonts */
/** @typedef {ReturnType<typeof dossier>} Dossier */
/** @typedef {Dossier["sections"][number]["rows"][number]} Row */
/** @typedef {{ row: string, pick: string, option: string, revision: number, timer: import("gpui-kit").Task | null }} Pending */

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

export default class PrioryBonjour extends View {
  /** @param {unknown} _props @param {import("gpui-kit").AsyncContext} cx */
  init(_props, cx) {
    /** @type {{ theme: string, client: ReturnType<typeof priory> } | null} */
    this.priory = null;
    /** @type {{ revision: number, report: any } | null} */
    this.data = null;
    /** Whether Priory answered that no task has written a report yet. */
    this.empty = false;
    /** @type {string | null} */
    this.error = null;
    this.updated = 0;
    this.width = 420;
    this.view = "all";
    /** @type {string | null} the selected row's id */
    this.selected = null;
    /** A pick waiting out its Undo, then in flight; nothing else is picked until it ends. */
    /** @type {Pending | null} */
    this.pending = null;
    /** @type {Map<string, string>} why a pick on a row was refused */
    this.notices = new Map();
    /** Why the last pick was refused, when its row has since left the view. */
    /** @type {string | null} */
    this.flash = null;
    this.focus = cx.focus_handle();
    /** @type {Fonts} */
    this.font = FONTS.indigo;
    this.scroll = ScrollHandle.new();
    this.refresh(cx);
  }

  /** Reads the report, then waits for the next revision and reads again; keeps the last good report on failure. */
  /** @param {import("gpui-kit").AsyncContext} cx */
  refresh(cx) {
    cx.spawn(async (cx) => {
      // The state revision last read; -1 makes the first wait return at once with the current one.
      let since = -1;
      for (;;) {
        try {
          this.priory ??= await connect();
          try {
            this.data = await this.priory.client.report();
            this.empty = false;
            since = this.data?.revision ?? since;
          } catch (error) {
            if (!message(error).endsWith(NO_REPORT)) throw error;
            this.data = null;
            this.empty = true;
          }
          this.updated = Date.now();
          this.error = null;
          cx.notify();
          since = await this.priory.client.wait(since);
        } catch (error) {
          this.priory = null;
          this.error = message(error);
          cx.notify();
          await cx.sleep(RETRY_MS);
        }
      }
    });
  }

  /** @param {number} [now] */
  model(now = Date.now()) {
    return this.data ? dossier(this.data, { now, view: this.view }) : null;
  }

  /** @param {Dossier} model */
  rows(model) {
    return model.sections.flatMap((section) => section.rows);
  }

  /** Scrolls the section that holds `row` into view; the greeting is the scroll region's first child. @param {Dossier} model @param {string} row */
  reveal(model, row) {
    const index = model.sections.findIndex((section) => section.rows.some((candidate) => candidate.id === row));
    if (index >= 0) this.scroll.scroll_to_item(index + 1);
  }

  /** @param {string | null} id @param {import("gpui-kit").Context} cx */
  select(id, cx) {
    this.focus.focus();
    this.selected = id;
    cx.notify();
  }

  /** @param {Dossier} model @param {import("gpui-kit").Context} cx */
  next(model, cx) {
    const id = nextPick(model, this.selected);
    if (!id) return;
    this.select(id, cx);
    this.reveal(model, id);
  }

  /** @param {Dossier} model @param {number} step @param {import("gpui-kit").Context} cx */
  move(model, step, cx) {
    const ids = this.rows(model).map((row) => row.id);
    const at = ids.indexOf(/** @type {string} */ (this.selected));
    const id = ids[Math.min(ids.length - 1, Math.max(0, at + step))];
    if (id === undefined) return;
    this.select(id, cx);
    this.reveal(model, id);
  }

  // A pick wakes the report's agent or requests a task, and the first choice stands, so it waits out an Undo.
  /** @param {Dossier} model @param {Row} row @param {string} option @param {import("gpui-kit").Context} cx */
  choose(model, row, option, cx) {
    if (this.pending || this.error || row.pick?.state !== "open") return;
    this.selected = row.id;
    this.notices.delete(row.id);
    this.flash = null;
    /** @type {Pending} */
    const pending = { row: row.id, pick: row.pick.id, option, revision: model.revision, timer: null };
    pending.timer = cx.timer.after(UNDO_MS, (cx) => {
      pending.timer = null;
      this.send(pending, cx);
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

  /** Records the pick with the report's revision; on failure re-reads the report and says why on the row. */
  /** @param {Pending} pending @param {import("gpui-kit").Context} cx */
  send(pending, cx) {
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        if (!this.priory) throw new Error("priory is not reachable");
        await this.priory.client.pick(pending.pick, pending.option, pending.revision);
        this.data = await this.priory.client.report();
      } catch (error) {
        /** @type {{ revision: number, report: any } | null} */
        let fresh = null;
        try {
          fresh = await this.priory?.client.report() ?? null;
        } catch {
          fresh = null;
        }
        if (fresh) this.data = fresh;
        const notice = refusal(pending, fresh, message(error));
        const model = this.model();
        if (model && this.rows(model).some((row) => row.id === pending.row)) this.notices.set(pending.row, notice);
        else this.flash = `${pending.option}: ${notice}`;
      }
      this.pending = null;
      cx.notify();
    });
  }

  /** @param {import("gpui-kit").KeyEvent} event @param {import("gpui-kit").Context} cx */
  key(event, cx) {
    if (event.key === "escape") return this.undo(cx);
    // A held key repeats only movement, so holding 1 never picks twice.
    if (event.is_held && !["j", "k", "up", "down"].includes(event.key)) return;
    if (event.modifiers?.control || event.modifiers?.platform || event.modifiers?.alt) return;
    const model = this.model();
    if (!model) return;
    if (event.key === "j" || event.key === "down") return this.move(model, 1, cx);
    if (event.key === "k" || event.key === "up") return this.move(model, -1, cx);
    if (event.key === "n") return this.next(model, cx);
    const row = this.rows(model).find((candidate) => candidate.id === this.selected);
    const option = row?.pick?.options.find((candidate) => candidate.key === event.key);
    if (row && option) this.choose(model, row, option.label, cx);
  }

  /** @param {import("gpui-kit").Context} cx */
  render(cx) {
    const theme = this.priory?.theme ?? "indigo";
    const t = /** @type {Palette} */ (palette(theme, cx.theme().appearance));
    this.font = FONTS[/** @type {keyof typeof FONTS} */ (theme)] ?? FONTS.indigo;
    const now = Date.now();
    const model = this.model(now);
    const wide = this.width >= RAIL.from;
    return v_flex().id("priory-bonjour").size_full().min_h_0().bg(t.bg).text_color(t.text).font_family(this.font.ui).text_size(TYPE.body[0])
      .track_focus(this.focus).on_key_down((event, cx) => this.key(event, cx))
      .on_size_change(({ width }, cx) => {
        if (Math.abs(width - this.width) < 1) return;
        this.width = width;
        cx.notify();
      })
      .child(this.toolbar(model, t, wide))
      .when(Boolean(this.error), (el) => el.child(this.offline(t, now)))
      .when(Boolean(this.flash), (el) => el.child(h_flex().flex_none().px(16).py(6).gap(8).items_center().border_b(1).border_color(t.line)
        .text_size(TYPE.small[0]).text_color(t["warn-text"]).child(div().flex_1().whitespace_normal().child(this.flash ?? ""))
        .child(this.secondary("flash-close", "Close", (cx) => { this.flash = null; cx.notify(); }, t))))
      .when(Boolean(model) && !wide, (el) => el.child(this.chips(/** @type {Dossier} */ (model), t)))
      .child(h_flex().flex_1().min_h_0()
        .when(Boolean(model) && wide, (el) => el.child(this.rail(/** @type {Dossier} */ (model), t)))
        .child(this.document(model, t, wide)));
  }

  /** The date, "n of m agreed" with its bar, and Next pick. */
  /** @param {Dossier | null} model @param {Palette} t @param {boolean} wide */
  toolbar(model, t, wide) {
    const progress = model?.progress;
    return h_flex().flex_none().h(PANE.head).px(16).gap(10).items_center().border_b(1).border_color(t.line)
      .child(div().font_family(this.font.mono).font_weight(600).text_size(TYPE.small[0]).child(model?.date ?? "Bonjour"))
      .when(wide && Boolean(model), (el) => el.child(div().text_size(TYPE.caption[0]).text_color(t["text-3"]).child(`Task ${model?.task}, revision ${model?.revision}`)))
      .child(div().flex_1())
      .when(Boolean(progress?.total), (el) => el.child(h_flex().flex_none().gap(8).items_center()
        .child(div().text_size(TYPE.caption[0]).text_color(t["text-2"]).child(progress?.label ?? ""))
        .child(div().w(wide ? 120 : 64).h(4).rounded(2).bg(t.raised)
          .child(div().h(4).rounded(2).bg(t.action).w(`${Math.round((progress?.agreed ?? 0) / (progress?.total || 1) * 100)}%`)))))
      .when(Boolean(model?.views[1].count), (el) => el.child(this.secondary("next-pick", "Next pick", (cx) => this.next(/** @type {Dossier} */ (model), cx), t)));
  }

  /** @param {Palette} t @param {number} now */
  offline(t, now) {
    const minutes = Math.round((now - this.updated) / 60_000);
    const last = this.data ? ` Last update ${minutes < 1 ? "under a minute" : `${minutes} min`} ago.` : "";
    return h_flex().flex_none().px(16).py(6).gap(6).items_start().text_size(TYPE.small[0]).text_color(t["warn-text"]).border_b(1).border_color(t.line)
      .child(div().flex_1().whitespace_normal().child(`Reconnecting to priory: ${this.error}.${last}`));
  }

  /** The views and the sections, as a list on the left from 1024 px. */
  /** @param {Dossier} model @param {Palette} t */
  rail(model, t) {
    /** @param {string} id @param {string} label @param {number | null} count @param {boolean} on @param {(cx: import("gpui-kit").Context) => void} action */
    const item = (id, label, count, on, action) => Button.new(id).w_full().h(PANE.chip).px(8).flex().items_center().gap(6).rounded(RADIUS.sm)
      .selected(on).bg(on ? t["action-soft"] : t.bg).text_color(on ? t["action-text"] : t.text).text_size(TYPE.small[0]).font_weight(on ? 600 : 500)
      .hover((s) => s.bg(on ? t["action-soft"] : t.raised)).focus((s) => s.border(2).border_color(t["action-text"]))
      .on_click((_event, cx) => action(cx))
      .child(div().flex_1().min_w_0().truncate().child(label))
      .when(count != null, (el) => el.child(div().text_size(TYPE.caption[0]).text_color(t["text-3"]).child(String(count))));
    return v_flex().id("bonjour-rail").flex_none().w(RAIL.width).min_h_0().overflow_y_scrollbar().p(8).gap(2).border_r(1).border_color(t.line)
      .children(model.views.map((view) => item(`view-${view.id}`, view.label, view.count, this.view === view.id, (cx) => this.show(view.id, cx))))
      .child(div().h(1).my(6).bg(t.line))
      .children(this.all(model).map((section) => item(`jump-${section.id}`, section.title, section.count, false, (cx) => this.jump(section.id, cx))));
  }

  /** The same list as chips under the toolbar, below 1024 px. */
  /** @param {Dossier} model @param {Palette} t */
  chips(model, t) {
    /** @param {string} id @param {string} label @param {boolean} on @param {(cx: import("gpui-kit").Context) => void} action */
    const chip = (id, label, on, action) => Button.new(id).h(PANE.chip).px(10).flex().flex_none().items_center().rounded(999)
      .selected(on).bg(on ? t.emph : t.bg).text_color(on ? t["on-emph"] : t["text-2"]).text_size(TYPE.small[0]).font_weight(500)
      .hover((s) => s.bg(on ? t.emph : t.raised)).focus((s) => s.border(2).border_color(t["action-text"]))
      .on_click((_event, cx) => action(cx)).child(label);
    return h_flex().flex_none().px(12).py(8).gap(4).flex_wrap().border_b(1).border_color(t.line)
      .children(model.views.map((view) => chip(`view-${view.id}`, view.count ? `${view.label} ${view.count}` : view.label, this.view === view.id, (cx) => this.show(view.id, cx))))
      .children(this.all(model).map((section) => chip(`jump-${section.id}`, section.title, false, (cx) => this.jump(section.id, cx))));
  }

  /** Every section, whichever view shows, so a jump can leave the Picks view. @param {Dossier} model */
  all(model) {
    return this.view === "all" || !this.data ? model.sections : dossier(this.data, { now: Date.now() }).sections;
  }

  /** @param {string} view @param {import("gpui-kit").Context} cx */
  show(view, cx) {
    this.view = view;
    this.scroll.scroll_to_item(0);
    cx.notify();
  }

  /** @param {string} section @param {import("gpui-kit").Context} cx */
  jump(section, cx) {
    this.view = "all";
    const model = /** @type {Dossier} */ (this.model());
    this.scroll.scroll_to_item(model.sections.findIndex((candidate) => candidate.id === section) + 1);
    cx.notify();
  }

  /** The greeting, then each section in the report's order. */
  /** @param {Dossier | null} model @param {Palette} t @param {boolean} wide */
  document(model, t, wide) {
    const pad = wide ? 24 : 16;
    if (!model) {
      const text = this.empty ? "No report yet. The factory's bonjour workflow writes one each morning." : this.error ? "" : "Reading the report…";
      return v_flex().id("bonjour-document").flex_1().p(pad).text_color(t["text-2"]).child(text);
    }
    return v_flex().id("bonjour-document").flex_1().min_w_0().min_h_0().overflow_y_scrollbar().track_scroll(this.scroll).px(pad).pt(pad).pb(32).gap(20)
      .child(v_flex().gap(4).max_w(760)
        .child(div().font_family(this.font.display).font_weight(700).text_size(wide ? 22 : 20).whitespace_normal().child(model.greeting))
        .child(div().text_size(TYPE.small[0]).text_color(t["text-2"]).whitespace_normal().child(model.since)))
      .when(!model.sections.length, (el) => el.child(div().text_color(t["text-2"]).child(this.view === "picks" ? "Every pick is agreed." : "This report has no sections.")))
      .children(model.sections.map((section) => v_flex().max_w(760).gap(6)
        .child(h_flex().gap(6).items_center().text_size(TYPE.small[0]).font_weight(650)
          .child(section.title).child(div().font_weight(400).text_color(t["text-3"]).child(String(section.count))))
        .children(section.rows.map((row) => row.pick ? this.pick(model, row, t) : this.line(row, t)))));
  }

  /** A row without a pick: its tile, key, text and owner or time, and its meta under it. Agent text draws as plain text. */
  /** @param {Row} row @param {Palette} t */
  line(row, t) {
    const selected = row.id === this.selected;
    return Button.new(`row-${row.id}`).w_full().min_h(PANE.row).px(8).py(8).flex().items_start().gap(10).rounded(RADIUS.md)
      .selected(selected).bg(selected ? t["action-soft"] : t.bg).hover((s) => s.bg(selected ? t["action-soft"] : t.raised))
      .focus((s) => s.border(2).border_color(t["action-text"]))
      .accessibility_label([row.tile, row.key, row.text, row.owner].filter(Boolean).join(", "))
      .on_click((_event, cx) => this.select(row.id, cx))
      .child(row.tile ? this.tile(row.tile, t) : h_flex().size(PANE.tile).flex_none().items_center().justify_center()
        .child(svg(ICON("file-text")).size(14).text_color(t["text-3"])))
      .child(v_flex().flex_1().min_w_0().gap(2)
        .child(h_flex().gap(8).items_start()
          .child(v_flex().flex_1().min_w_0().gap(2)
            .when(Boolean(row.key), (el) => el.child(div().font_family(this.font.mono).text_size(TYPE.caption[0]).text_color(t["text-2"]).child(row.key ?? "")))
            .child(div().whitespace_normal().line_height(TYPE.body[1]).font_weight(500).child(row.text)))
          .when(Boolean(row.owner || row.time), (el) => el.child(div().flex_none().text_size(TYPE.caption[0])
            .text_color(row.owner ? t["warn-text"] : t["text-3"]).font_weight(row.owner ? 600 : 400).child(row.owner ?? row.time ?? ""))))
        .when(Boolean(row.meta), (el) => el.child(div().whitespace_normal().text_size(TYPE.caption[0]).text_color(t["text-3"]).child(row.meta ?? ""))));
  }

  /** @param {string} state @param {Palette} t */
  tile(state, t) {
    const [fill, ink, icon] = TILES[state] ?? ["tile-muted", "on-tile", null];
    return h_flex().size(PANE.tile).flex_none().items_center().justify_center().rounded(RADIUS.tile).bg(t[fill]).accessibility_label(state)
      .child(icon ? svg(ICON(icon)).size(14).text_color(t[ink]) : div().size(8).rounded(4).bg(t[ink]));
  }

  /** A row with a pick: a card with a segmented control while open, one line once agreed. */
  /** @param {Dossier} model @param {Row} row @param {Palette} t */
  pick(model, row, t) {
    const pick = /** @type {NonNullable<Row["pick"]>} */ (row.pick);
    const selected = row.id === this.selected;
    const notice = this.notices.get(row.id);
    if (pick.state !== "open") {
      return h_flex().w_full().min_h(PANE.control + 8).px(12).py(6).gap(8).items_center().rounded(RADIUS.md).border(1).border_color(t.line)
        .bg(selected ? t["action-soft"] : t.bg)
        .child(svg(ICON("check")).size(14).flex_none().text_color(t["ok-text"]))
        .child(div().flex_none().max_w("40%").truncate().font_family(row.key ? this.font.mono : this.font.ui).text_size(TYPE.small[0]).text_color(t["text-2"]).child(row.key ?? row.text))
        .child(div().flex_none().font_weight(600).child(pick.chosen ?? ""))
        .child(div().flex_1().min_w_0().truncate().text_size(TYPE.caption[0]).text_color(t["text-3"]).child(pick.note ?? ""));
    }
    const pending = this.pending?.row === row.id ? this.pending : null;
    const busy = Boolean(this.pending) || Boolean(this.error);
    return v_flex().w_full().p(12).gap(6).rounded(RADIUS.md).border(1).border_color(selected ? t["action-soft"] : t.line).bg(selected ? t["action-soft"] : t.surface)
      .when(Boolean(row.key), (el) => el.child(div().font_family(this.font.mono).text_size(TYPE.caption[0]).text_color(t["text-2"]).child(row.key ?? "")))
      .child(div().whitespace_normal().line_height(TYPE.body[1]).font_weight(600).child(row.text))
      .when(Boolean(row.meta), (el) => el.child(div().whitespace_normal().text_size(TYPE.caption[0]).text_color(t["text-3"]).child(row.meta ?? "")))
      .when(Boolean(notice), (el) => el.child(div().whitespace_normal().text_size(TYPE.small[0]).text_color(t["warn-text"]).child(notice ?? "")))
      .child(h_flex().gap(8).items_center().flex_wrap()
        .child(h_flex().flex_none().p(2).gap(2).rounded(RADIUS.sm + 2).border(1).border_color(t["control-line"]).bg(t.bg)
          .children(pick.options.map((option) => {
            const on = pending?.option === option.label;
            const hint = !pending && option.recommended;
            return Button.new(`pick-${row.id}-${option.key || option.label}`).h(PANE.chip - 4).px(10).flex().items_center().gap(4).rounded(RADIUS.sm)
              .selected(on).disabled(busy && !on).opacity(busy && !on ? 0.5 : 1)
              .bg(on ? t.emph : t.bg).text_color(on ? t["on-emph"] : hint ? t["action-text"] : t.text)
              .text_size(TYPE.small[0]).font_weight(on || hint ? 600 : 500)
              .when(!busy, (el) => el.hover((s) => s.bg(t.raised))).focus((s) => s.border(2).border_color(t["action-text"]))
              .accessibility_label([option.label, option.recommended && "recommended", option.key && `key ${option.key}`].filter(Boolean).join(", "))
              .on_click((_event, cx) => this.choose(model, row, option.label, cx))
              .when(hint, (el) => el.child(svg(ICON("check")).size(13)))
              .child(option.label);
          })))
        .when(Boolean(pick.note) && !pending, (el) => el.child(div().text_size(TYPE.caption[0]).text_color(t["text-3"]).child(pick.note ?? "")))
        .when(Boolean(pending), (el) => el.child(h_flex().flex_1().gap(8).items_center().justify_end().text_size(TYPE.small[0])
          .child(div().child(pending?.timer ? `Picking ${pending.option}` : `Sending ${pending?.option}…`))
          .when(Boolean(pending?.timer), (el) => el.child(this.secondary("undo", "Undo", (cx) => this.undo(cx), t))))));
  }

  /** @param {string} id @param {string} label @param {(cx: import("gpui-kit").Context) => void} action @param {Palette} t */
  secondary(id, label, action, t) {
    return Button.new(id).h(PANE.control - 4).px(12).flex().flex_none().items_center().rounded(RADIUS.card).border(1).border_color(t["control-line"])
      .bg(t.surface).text_color(t.text).text_size(TYPE.small[0]).font_weight(600).hover((s) => s.bg(t.raised))
      .focus((s) => s.border(2).border_color(t["action-text"])).on_click((_event, cx) => action(cx)).child(label);
  }
}
