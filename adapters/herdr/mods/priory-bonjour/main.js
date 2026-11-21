import { View, div, svg } from "gpui-kit";
import { h_flex, v_flex, Button, Input, InputState, ScrollHandle, TextView } from "gpui-base";
import { appHome, exec, home as herdrHome, readTextFile } from "herdr";
import { follow, priory, spansHtml } from "./priory.js";
import { FONTS, palette, STRUCTURE as S } from "./tokens.js";
import { around, dossier, nextPick, refusal } from "./view.js";

// gpui-kit reads a bare line height as a multiplier, so a line height is given as a length.
const LINE = (/** @type {number[]} */ [, height]) => /** @type {`${number}px`} */ (`${height}px`);
// From this width the sections list sits on the left (DESIGN.md, Bonjour).
const RAIL = { from: 1024, width: 200 };
const DOCUMENT = 760;
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
/** @typedef {{ revision: number, report: any }} Read */
/** @typedef {ReturnType<typeof dossier>} Dossier */
/** @typedef {Dossier["sections"][number]} Section */
/** @typedef {Section["rows"][number]} Row */
/** @typedef {{ task: number, row: string, pick: string, option: string, from: string | null, revision: number, timer: import("gpui-kit").Task | null }} Pending */
/** @typedef {["greeting"] | ["empty"] | ["section", Section] | ["row", Row]} Item */

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
  const client = priory({ exec, binary, on, factory: config.factory ?? null });
  // Links are optional: without a factory priory can read, the panel shows its text unlinked.
  const links = await client.links().then((read) => read.links, () => []);
  return { theme: config.theme ?? "indigo", client, links };
}

/**
 * Text with its autolinks; plain text stays a plain child. Linked text keeps its press, so a link
 * opens without selecting the row Button around it.
 * @param {string} id @param {{ text: string, url?: string }[]} spans
 */
const linked = (id, spans) => (spans.some((span) => span.url)
  ? div().on_mouse_down("left", (_event, cx) => cx.stop_propagation()).child(TextView.html(id, spansHtml(spans)))
  : spans.map((span) => span.text).join(""));

const message = (/** @type {unknown} */ error) => (error instanceof Error ? error.message : String(error));

/** The scroll region's direct children in order, so a row or a section can be scrolled to by its index. @param {Dossier} model */
function items(model) {
  /** @type {Item[]} */
  const list = [["greeting"]];
  if (!model.sections.length) list.push(["empty"]);
  for (const section of model.sections) {
    list.push(["section", section]);
    for (const row of section.rows) list.push(["row", row]);
  }
  return list;
}

/** @param {Item} item */
const itemId = (item) => (item[0] === "section" ? `section:${item[1].id}` : item[0] === "row" ? item[1].id : item[0]);

export default class PrioryBonjour extends View {
  /** @param {unknown} _props @param {import("gpui-kit").AsyncContext} cx */
  init(_props, cx) {
    /** @type {{ theme: string, client: ReturnType<typeof priory> } | null} */
    this.priory = null;
    /** @type {Read | null} */
    this.data = null;
    /** @type {{ pattern: string, url: string }[]} */
    this.links = [];
    /** Whether Priory answered that no task has written a report yet. */
    this.empty = false;
    /** @type {string | null} */
    this.error = null;
    this.updated = 0;
    this.width = 420;
    this.view = "all";
    /** @type {number | null} the task whose report shows; null follows the one written last */
    this.shown = null;
    /** @type {{ task: number }[]} `priory report list`'s reports, newest first */
    this.list = [];
    /** @type {string | null} the selected row's id */
    this.selected = null;
    /** A pick waiting out its Undo, then in flight; nothing else is picked until it ends. */
    /** @type {Pending | null} */
    this.pending = null;
    /** @type {string | null} the row whose agreed pick Change reopened */
    this.changing = null;
    /** @type {Map<string, string>} why a pick on a row was refused */
    this.notices = new Map();
    /** @type {string | null} a pick's outcome when its row has left the view */
    this.flash = null;
    /** @type {number | null} the thread the comment field replies in; null starts a thread */
    this.replyTo = null;
    this.posting = false;
    this.typing = false;
    this.draft = InputState.new({ placeholder: "Comment on this row" });
    this.draft.on("change", (_event, cx) => cx.notify());
    this.draft.on("submit", (_event, cx) => this.post(cx));
    this.draft.on("focus", (_event, cx) => { this.typing = true; cx.notify(); });
    this.draft.on("blur", (_event, cx) => { this.typing = false; cx.notify(); });
    /** Whether the search field shows; its text filters the rows. */
    this.searching = false;
    this.query = InputState.new({ placeholder: "Search this report" });
    this.query.on("change", (_event, cx) => cx.notify());
    this.query.on("focus", (_event, cx) => { this.typing = true; cx.notify(); });
    this.query.on("blur", (_event, cx) => { this.typing = false; cx.notify(); });
    this.focus = cx.focus_handle();
    /** @type {Fonts} */
    this.font = FONTS.indigo;
    this.scroll = ScrollHandle.new();
    this.refresh(cx);
  }

  /** Reads the report, then waits for the next revision and reads again; keeps the last good report on failure. */
  /** @param {import("gpui-kit").AsyncContext} cx */
  refresh(cx) {
    /** @type {ReturnType<typeof priory>} the client of the last read, which its wait uses */
    let client;
    cx.spawn((cx) => follow({
      // With no report yet there is no report revision, so the wait goes on from the last one it returned.
      read: async () => {
        this.priory ??= await connect();
        this.links = this.priory.links;
        client = this.priory.client;
        // The arrows are optional: a priory without `report list` leaves them off.
        this.list = await client.reports().then((read) => read.reports, () => []);
        try {
          // An arrow pressed while this read was in flight decides what shows.
          const asked = this.shown;
          const read = await client.report(asked);
          if (this.shown === asked) this.accept(read);
        } catch (error) {
          if (!message(error).endsWith(NO_REPORT)) throw error;
          this.accept(null);
        }
        this.updated = Date.now();
        this.error = null;
        cx.notify();
        return this.data?.revision ?? null;
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

  /**
   * Takes a read unless it is older than the one shown, as a pick's read can finish after the loop's.
   * A pick still waiting out its Undo is cancelled when the report no longer offers it, since its
   * Undo would no longer show.
   * @param {Read | null} read null when no task has a report
   */
  accept(read) {
    if (read && this.data && read.revision < this.data.revision) return;
    // A read that started before an arrow moved to another report.
    if (read && this.shown != null && read.report.task !== this.shown) return;
    if (read?.report.task !== this.data?.report.task) {
      this.selectRow(null);
      this.notices.clear();
    }
    this.data = read;
    this.empty = !read;
    const pending = this.pending;
    if (!pending?.timer) return;
    const still = read && read.report.task === pending.task
      && this.rows(dossier(read, { now: Date.now() })).some((row) => row.id === pending.row && row.pick?.id === pending.pick
        && row.pick.state !== "applied" && row.pick.chosen === pending.from);
    if (still) return;
    pending.timer.cancel();
    this.pending = null;
    this.flash = `${pending.option} was not sent: the report changed before it went.`;
  }

  /** @param {number} [now] */
  model(now = Date.now()) {
    const query = this.searching ? this.query.value() : "";
    return this.data ? dossier(this.data, { now, view: this.view, links: this.links, query }) : null;
  }

  /** @param {Dossier} model */
  rows(model) {
    return model.sections.flatMap((section) => section.rows);
  }

  /** @param {Dossier} model @param {string} id a row's id, or `section:` and a section's id */
  reveal(model, id) {
    const index = items(model).findIndex((item) => itemId(item) === id);
    if (index >= 0) this.scroll.scroll_to_item(index);
  }

  /** Shows task `task`'s report; the newest one is followed as it changes. @param {number} task @param {import("gpui-kit").Context} cx */
  go(task, cx) {
    this.shown = task === this.list[0]?.task ? null : task;
    this.scroll.scroll_to_item(0);
    const shown = this.shown;
    cx.spawn(async (cx) => {
      try {
        const read = await this.priory?.client.report(shown);
        if (read && this.shown === shown) this.accept(read);
      } catch (error) {
        this.flash = `Could not read that report: ${message(error)}.`;
      }
      cx.notify();
    });
  }

  /** @param {Dossier} model @param {string | null} id @param {import("gpui-kit").Context} cx */
  select(model, id, cx) {
    this.focus.focus();
    this.selectRow(id);
    if (id) this.reveal(model, id);
    cx.notify();
  }

  /** Selects a row; a comment belongs to the row it was typed for, so another row clears the field. @param {string | null} id */
  selectRow(id) {
    if (id !== this.selected) {
      this.replyTo = null;
      this.draft.set_value("");
    }
    this.selected = id;
  }

  /** @param {Dossier} model @param {number} step @param {import("gpui-kit").Context} cx */
  move(model, step, cx) {
    const ids = this.rows(model).map((row) => row.id);
    const at = ids.indexOf(/** @type {string} */ (this.selected));
    const id = ids[Math.min(ids.length - 1, Math.max(0, at + step))];
    if (id !== undefined) this.select(model, id, cx);
  }

  // A pick wakes the report's agent or requests a task, and the first choice stands, so it waits out an Undo.
  /** @param {Dossier} model @param {Row} row @param {string} option @param {import("gpui-kit").Context} cx */
  choose(model, row, option, cx) {
    const pick = row.pick;
    const reopened = pick?.state === "agreed" && this.changing === row.id;
    if (this.pending || this.error || !pick || (pick.state !== "open" && !reopened) || option === pick.chosen) return;
    this.selectRow(row.id);
    this.notices.delete(row.id);
    this.flash = null;
    /** @type {Pending} */
    const pending = { task: model.task, row: row.id, pick: pick.id, option, from: pick.chosen, revision: model.revision, timer: null };
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

  /** Records the pick with the report's revision; on refusal re-reads the report and says why on the row. */
  /** @param {Pending} pending @param {import("gpui-kit").Context} cx */
  send(pending, cx) {
    cx.notify();
    cx.spawn(async (cx) => {
      const client = this.priory?.client;
      try {
        if (!client) throw new Error("priory is not reachable");
        await client.pick(pending.pick, pending.option, pending.revision);
      } catch (error) {
        /** @type {Read | null} */
        let fresh = null;
        try {
          fresh = await client?.report(pending.task) ?? null;
        } catch {
          fresh = null;
        }
        if (fresh?.report.task === this.data?.report.task) this.accept(fresh);
        const notice = refusal(pending, fresh, message(error));
        const model = this.model();
        if (model?.task === pending.task && this.rows(model).some((row) => row.id === pending.row)) this.notices.set(pending.row, notice);
        else this.flash = `${pending.option}: ${notice}`;
        this.pending = null;
        this.changing = null;
        return cx.notify();
      }
      // Priory took the pick; a failed read here leaves the read loop to show it.
      try {
        this.accept(await client.report());
      } catch {
        this.flash = `${pending.option} was sent. The report shows it once priory answers again.`;
      }
      this.pending = null;
      this.changing = null;
      cx.notify();
    });
  }

  /** Sends the comment field as a new thread on the selected row, or as a reply in `replyTo`. */
  /** @param {import("gpui-kit").Context} cx */
  post(cx) {
    const model = this.model();
    const row = model && this.rows(model).find((candidate) => candidate.id === this.selected);
    const text = this.draft.value().trim();
    if (!model || !row || !text || (this.replyTo == null && !this.threadable(row))) return;
    const [task, target, replyTo, revision] = [model.task, row.target, this.replyTo, model.revision];
    this.remark(row, (client) => (replyTo == null ? client.comment(target, text, revision) : client.reply(replyTo, text)), cx, () => {
      // The person may have moved on or typed more while it was in flight.
      const same = this.data?.report.task === task && this.selected === row.id && this.replyTo === replyTo;
      if (!same || this.draft.value().trim() !== text) return;
      this.draft.set_value("");
      this.replyTo = null;
    });
  }

  /**
   * Sends one comment verb, then reads the report again; a refusal says why under the row.
   * @param {Row} row @param {(client: ReturnType<typeof priory>) => Promise<string>} call
   * @param {import("gpui-kit").Context} cx @param {() => void} [sent]
   */
  remark(row, call, cx, sent = () => {}) {
    if (this.posting) return;
    this.posting = true;
    this.notices.delete(row.id);
    cx.notify();
    cx.spawn(async (cx) => {
      const client = this.priory?.client;
      try {
        if (!client) throw new Error("priory is not reachable");
        await call(client);
      } catch (error) {
        this.notices.set(row.id, `Not sent: ${message(error)}.`);
        this.posting = false;
        return cx.notify();
      }
      sent();
      // Priory took it; a failed read here leaves the read loop to show it.
      try {
        this.accept(await client.report());
      } catch {
        this.flash = "Sent. The report shows it once priory answers again.";
      }
      this.posting = false;
      cx.notify();
    });
  }

  /** @param {import("gpui-kit").KeyEvent} event @param {import("gpui-kit").Context} cx */
  key(event, cx) {
    // The comment field keeps its letters; Escape leaves it.
    if (this.typing) {
      if (event.key === "escape") this.focus.focus();
      return;
    }
    if (event.key === "escape") return this.undo(cx);
    // A held key repeats only movement, so holding 1 never picks twice.
    if (event.is_held && !["j", "k", "up", "down"].includes(event.key)) return;
    if (event.modifiers?.control || event.modifiers?.platform || event.modifiers?.alt) return;
    const model = this.model();
    if (!model) return;
    if (event.key === "j" || event.key === "down") return this.move(model, 1, cx);
    if (event.key === "k" || event.key === "up") return this.move(model, -1, cx);
    if (event.key === "n") return this.select(model, nextPick(model, this.selected) ?? this.selected, cx);
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
    return v_flex().id("priory-bonjour").size_full().min_h_0().bg(t.bg).text_color(t.text).font_family(this.font.ui).text_size(S["type-body"][0])
      .track_focus(this.focus).on_key_down((event, cx) => this.key(event, cx))
      .on_size_change(({ width }, cx) => {
        if (Math.abs(width - this.width) < 1) return;
        this.width = width;
        cx.notify();
      })
      .child(this.toolbar(model, t, wide))
      .when(Boolean(this.error), (el) => el.child(this.offline(t, now)))
      .when(Boolean(this.flash), (el) => el.child(h_flex().flex_none().px(S["space-4"]).py(6).gap(S["space-2"]).items_center().border_b(1).border_color(t.line)
        .text_size(S["type-small"][0]).text_color(t["warn-text"]).child(div().flex_1().whitespace_normal().child(this.flash ?? ""))
        .child(this.secondary("flash-close", "Close", (cx) => { this.flash = null; cx.notify(); }, t))))
      .when(Boolean(model) && this.searching, (el) => el.child(this.searchbar(/** @type {Dossier} */ (model), t)))
      .when(Boolean(model) && !wide, (el) => el.child(this.chips(/** @type {Dossier} */ (model), t)))
      .child(h_flex().flex_1().min_h_0()
        .when(Boolean(model) && wide, (el) => el.child(this.rail(/** @type {Dossier} */ (model), t)))
        .child(this.document(model, t, wide)));
  }

  /** @param {Dossier | null} model @param {Palette} t @param {boolean} wide */
  toolbar(model, t, wide) {
    const progress = model?.progress;
    const { older, newer } = model ? around(this.list, model.task) : { older: null, newer: null };
    /** @param {string} id @param {string} label @param {string} icon @param {number | null} task */
    const arrow = (id, label, icon, task) => Button.new(id).size(S["control-fine"] - 4).flex().flex_none().items_center().justify_center().rounded(S["radius-sm"])
      .disabled(task == null).opacity(task == null ? 0.4 : 1).accessibility_label(label)
      .when(task != null, (el) => el.hover((s) => s.bg(t.raised))).focus((s) => s.border(2).border_color(t["action-text"]))
      .on_click((_event, cx) => { if (task != null) this.go(task, cx); })
      .child(svg(ICON(icon)).size(14).text_color(t["text-2"]));
    return h_flex().flex_none().h(S["pane-head"]).px(S["space-4"]).gap(10).items_center().border_b(1).border_color(t.line)
      .child(h_flex().flex_none().gap(2).items_center()
        .when(Boolean(model), (el) => el.child(arrow("report-older", "Older report", "chevron-left", older)))
        .child(div().font_family(this.font.mono).font_weight(600).text_size(S["type-small"][0]).child(model?.date ?? "Bonjour"))
        .when(Boolean(model), (el) => el.child(arrow("report-newer", "Newer report", "chevron-right", newer))))
      .when(wide && Boolean(model), (el) => el.child(div().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(`Task ${model?.task}, revision ${model?.revision}`)))
      .child(div().flex_1())
      .when(Boolean(progress?.total), (el) => el.child(h_flex().flex_none().gap(S["space-2"]).items_center()
        .child(div().text_size(S["type-caption"][0]).text_color(t["text-2"]).child(progress?.label ?? ""))
        .child(div().w(wide ? 120 : 64).h(4).rounded(2).bg(t.raised)
          .child(div().h(4).rounded(2).bg(t.action).w(`${Math.round((progress?.agreed ?? 0) / (progress?.total || 1) * 100)}%`)))))
      .when(Boolean(model?.views[1].count), (el) => el.child(this.secondary("next-pick", "Next pick", (cx) => {
        const current = /** @type {Dossier} */ (model);
        this.select(current, nextPick(current, this.selected) ?? this.selected, cx);
      }, t)))
      .when(Boolean(model), (el) => el.child(this.secondary("report-search", this.searching ? "Close search" : "Search", (cx) => this.search(cx), t)));
  }

  /** Opens the search field, or closes it and clears its text. @param {import("gpui-kit").Context} cx */
  search(cx) {
    this.searching = !this.searching;
    this.query.set_value("");
    if (this.searching) this.query.focus();
    else this.focus.focus();
    cx.notify();
  }

  /** @param {Dossier} model @param {Palette} t */
  searchbar(model, t) {
    return h_flex().id("report-query").flex_none().px(S["space-4"]).py(6).gap(S["space-2"]).items_center().border_b(1).border_color(t.line)
      .child(div().flex_1().min_w_0().h(S["control-fine"]).px(8).rounded(S["radius-sm"]).border(1).border_color(t["control-line"])
        .child(Input.new(this.query).h_full().w_full().text_size(S["type-small"][0]).text_color(t.text)))
      .when(model.found != null, (el) => el.child(div().flex_none().text_size(S["type-caption"][0]).text_color(t["text-2"]).child(`${model.found} found`)));
  }

  /** @param {Palette} t @param {number} now */
  offline(t, now) {
    const minutes = Math.round((now - this.updated) / 60_000);
    const last = this.data ? ` Last update ${minutes < 1 ? "under a minute" : `${minutes} min`} ago.` : "";
    return h_flex().flex_none().px(S["space-4"]).py(6).gap(6).items_start().text_size(S["type-small"][0]).text_color(t["warn-text"]).border_b(1).border_color(t.line)
      .child(div().flex_1().whitespace_normal().child(`Reconnecting to priory: ${this.error}.${last}`));
  }

  /** @param {Dossier} model @param {Palette} t */
  rail(model, t) {
    /** @param {string} id @param {string} label @param {number | null} count @param {boolean} on @param {(cx: import("gpui-kit").Context) => void} action */
    const item = (id, label, count, on, action) => Button.new(id).w_full().h(S["pane-chip"]).px(S["space-2"]).flex().items_center().gap(6).rounded(S["radius-sm"])
      .selected(on).bg(on ? t["action-soft"] : t.bg).text_color(on ? t["action-text"] : t.text).text_size(S["type-small"][0]).font_weight(on ? 600 : 500)
      .hover((s) => s.bg(on ? t["action-soft"] : t.raised)).focus((s) => s.border(2).border_color(t["action-text"]))
      .on_click((_event, cx) => action(cx))
      .child(div().flex_1().min_w_0().truncate().child(label))
      .when(count != null, (el) => el.child(div().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(String(count))));
    return v_flex().id("bonjour-rail").flex_none().w(RAIL.width).min_h_0().overflow_y_scrollbar().p(S["space-2"]).gap(2).border_r(1).border_color(t.line)
      .children(model.views.map((view) => item(`view-${view.id}`, view.label, view.count, this.view === view.id, (cx) => this.show(view.id, cx))))
      .child(div().h(1).my(6).bg(t.line))
      .children(this.every(model).map((section) => item(`jump-${section.id}`, section.title, section.count, false, (cx) => this.jump(section.id, cx))));
  }

  /** @param {Dossier} model @param {Palette} t */
  chips(model, t) {
    /** @param {string} id @param {string} label @param {boolean} on @param {(cx: import("gpui-kit").Context) => void} action */
    const chip = (id, label, on, action) => Button.new(id).h(S["pane-chip"]).px(10).flex().flex_none().items_center().rounded(S["radius-full"])
      .selected(on).bg(on ? t.emph : t.bg).text_color(on ? t["on-emph"] : t["text-2"]).text_size(S["type-small"][0]).font_weight(500)
      .hover((s) => s.bg(on ? t.emph : t.raised)).focus((s) => s.border(2).border_color(t["action-text"]))
      .on_click((_event, cx) => action(cx)).child(label);
    return h_flex().flex_none().px(S["space-3"]).py(S["space-2"]).gap(S["space-1"]).flex_wrap().border_b(1).border_color(t.line)
      .children(model.views.map((view) => chip(`view-${view.id}`, view.count ? `${view.label} ${view.count}` : view.label, this.view === view.id, (cx) => this.show(view.id, cx))))
      .children(this.every(model).map((section) => chip(`jump-${section.id}`, section.title, false, (cx) => this.jump(section.id, cx))));
  }

  /** Every section, whichever view shows, so a jump can leave the Picks view. @param {Dossier} model */
  every(model) {
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
    const model = this.model();
    if (model) this.reveal(model, `section:${section}`);
    cx.notify();
  }

  /** @param {Dossier | null} model @param {Palette} t @param {boolean} wide */
  document(model, t, wide) {
    const pad = wide ? S["space-5"] : S["space-4"];
    if (!model) {
      const text = this.empty ? "No report yet. The factory's bonjour workflow writes one each morning." : this.error ? "" : "Reading the report…";
      return v_flex().id("bonjour-document").flex_1().p(pad).text_color(t["text-2"]).child(text);
    }
    return v_flex().id("bonjour-document").flex_1().min_w_0().min_h_0().overflow_y_scrollbar().track_scroll(this.scroll).px(pad).pt(pad).pb(32).gap(6)
      .children(items(model).map((item) => {
        if (item[0] === "greeting") {
          return v_flex().gap(S["space-1"]).max_w(DOCUMENT).pb(S["space-3"])
            .child(div().font_family(this.font.display).font_weight(700).text_size(S["type-hero"][0]).line_height(LINE(S["type-hero"])).whitespace_normal().child(model.greeting))
            .child(div().text_size(S["type-small"][0]).text_color(t["text-2"]).whitespace_normal().child(model.since));
        }
        if (item[0] === "empty") return div().text_color(t["text-2"]).child(model.found != null
          ? `Nothing in this report holds “${this.query.value().trim()}”.`
          : this.view === "picks" ? "Every pick is agreed." : "This report has no sections.");
        if (item[0] === "section") {
          return h_flex().max_w(DOCUMENT).pt(S["space-3"]).gap(6).items_center().text_size(S["type-small"][0]).font_weight(650)
            .child(item[1].title).child(div().font_weight(400).text_color(t["text-3"]).child(String(item[1].count)));
        }
        const row = item[1];
        return v_flex().max_w(DOCUMENT).child(row.pick ? this.pick(model, row, t) : this.line(model, row, t))
          .when(row.threads.length > 0 || (row.id === this.selected && (this.threadable(row) || this.replyTo != null)), (el) => el.child(this.threads(model, row, t)));
      }));
  }

  /**
   * Whether a row takes a new thread: Priory gives a row id's comment to the newest report holding
   * it, so only the newest report, which the panel follows, takes one.
   * @param {Row} row
   */
  threadable(row) {
    const newest = this.list[0]?.task;
    return row.target != null && this.shown == null && (newest == null || this.data?.report.task === newest);
  }

  /** @param {Row} row @param {Palette} t */
  side(row, t) {
    return div().flex_none().text_size(S["type-caption"][0]).text_color(row.owner ? t["warn-text"] : t["text-3"])
      .font_weight(row.owner ? 600 : 400).child(row.owner ?? row.time ?? "");
  }

  /** @param {Dossier} model @param {Row} row @param {Palette} t */
  line(model, row, t) {
    const selected = row.id === this.selected;
    return Button.new(`row-${row.id}`).w_full().min_h(S["pane-row"]).px(S["space-2"]).py(S["space-2"]).flex().items_start().gap(10).rounded(S["radius-md"])
      .selected(selected).bg(selected ? t["action-soft"] : t.bg).hover((s) => s.bg(selected ? t["action-soft"] : t.raised))
      .focus((s) => s.border(2).border_color(t["action-text"]))
      .accessibility_label([row.tile, row.key, row.text, row.owner].filter(Boolean).join(", "))
      .on_click((_event, cx) => this.select(model, row.id, cx))
      .child(row.tile ? this.tile(row.tile, t) : h_flex().size(S["pane-tile"]).flex_none().items_center().justify_center()
        .child(svg(ICON("file-text")).size(14).text_color(t["text-3"])))
      .child(v_flex().flex_1().min_w_0().gap(2)
        .child(h_flex().gap(S["space-2"]).items_start()
          .child(v_flex().flex_1().min_w_0().gap(2)
            .when(Boolean(row.key), (el) => el.child(div().font_family(this.font.mono).text_size(S["type-caption"][0]).text_color(t["text-2"]).child(row.key ?? "")))
            .child(div().whitespace_normal().line_height(LINE(S["type-body"])).font_weight(500).child(linked(`text-${row.id}`, row.textSpans))))
          .when(Boolean(row.owner || row.time), (el) => el.child(this.side(row, t))))
        .when(Boolean(row.meta), (el) => el.child(div().whitespace_normal().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(row.meta ?? ""))));
  }

  /** @param {string} state @param {Palette} t */
  tile(state, t) {
    const [fill, ink, icon] = TILES[state] ?? ["tile-muted", "on-tile", null];
    return h_flex().size(S["pane-tile"]).flex_none().items_center().justify_center().rounded(S["radius-tile"]).bg(t[fill]).accessibility_label(state)
      .child(icon ? svg(ICON(icon)).size(14).text_color(t[ink]) : div().size(8).rounded(4).bg(t[ink]));
  }

  /** @param {string | undefined} notice @param {Palette} t */
  notice(notice, t) {
    return div().whitespace_normal().text_size(S["type-small"][0]).text_color(t["warn-text"]).child(notice ?? "");
  }

  /** An open pick is a card with a segmented control; an agreed one folds to one line. */
  /** @param {Dossier} model @param {Row} row @param {Palette} t */
  pick(model, row, t) {
    const pick = /** @type {NonNullable<Row["pick"]>} */ (row.pick);
    const selected = row.id === this.selected;
    const notice = this.notices.get(row.id);
    // A pick card selects nothing on a press, so a row with an id offers Comment to reach the field.
    const commentable = !selected && this.threadable(row);
    const reopened = pick.state === "agreed" && this.changing === row.id;
    if (pick.state !== "open" && !reopened) {
      return v_flex().w_full().px(S["space-3"]).py(6).gap(S["space-1"]).rounded(S["radius-md"]).border(1).border_color(t.line).bg(selected ? t["action-soft"] : t.bg)
        .child(h_flex().min_h(S["control-fine"] - 4).gap(S["space-2"]).items_center()
          .child(svg(ICON("check")).size(14).flex_none().text_color(t["ok-text"]))
          .child(div().flex_none().max_w("40%").truncate().font_family(row.key ? this.font.mono : this.font.ui).text_size(S["type-small"][0]).text_color(t["text-2"]).child(row.key ?? row.text))
          .child(div().flex_none().font_weight(600).child(pick.chosen ?? ""))
          .child(div().flex_1().min_w_0().truncate().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(pick.note ?? ""))
          .when(pick.state === "agreed" && !this.pending && !this.error, (el) => el.child(this.secondary(`change-${row.id}`, "Change", (cx) => {
            this.changing = row.id;
            cx.notify();
          }, t)))
          .when(commentable, (el) => el.child(this.secondary(`comment-${row.id}`, "Comment", (cx) => this.select(model, row.id, cx), t))))
        .when(Boolean(notice), (el) => el.child(this.notice(notice, t)));
    }
    const pending = this.pending?.row === row.id ? this.pending : null;
    const busy = Boolean(this.pending) || Boolean(this.error);
    return v_flex().w_full().p(S["space-3"]).gap(6).rounded(S["radius-md"]).border(1).border_color(selected ? t["action-soft"] : t.line).bg(selected ? t["action-soft"] : t.surface)
      .when(Boolean(row.tile || row.key || row.owner || row.time), (el) => el.child(h_flex().gap(S["space-2"]).items_center()
        .when(Boolean(row.tile), (el) => el.child(this.tile(/** @type {string} */ (row.tile), t)))
        .child(div().flex_1().min_w_0().truncate().font_family(this.font.mono).text_size(S["type-caption"][0]).text_color(t["text-2"]).child(row.key ?? ""))
        .when(Boolean(row.owner || row.time), (el) => el.child(this.side(row, t)))))
      .child(div().whitespace_normal().line_height(LINE(S["type-body"])).font_weight(600).child(linked(`text-${row.id}`, row.textSpans)))
      .when(Boolean(row.meta), (el) => el.child(div().whitespace_normal().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(row.meta ?? "")))
      .when(Boolean(notice), (el) => el.child(this.notice(notice, t)))
      .child(h_flex().gap(S["space-2"]).items_center().flex_wrap()
        .child(h_flex().min_w_0().flex_wrap().p(2).gap(2).rounded(S["radius-sm"] + 2).border(1).border_color(t["control-line"]).bg(t.bg)
          .children(pick.options.map((option) => {
            const on = pending ? pending.option === option.label : option.label === pick.chosen;
            const hint = !pending && !reopened && option.recommended;
            return Button.new(`pick-${row.id}-${option.key || option.label}`).h(S["pane-chip"] - 4).px(10).flex().items_center().gap(S["space-1"]).rounded(S["radius-sm"])
              .selected(on).disabled(busy && !on).opacity(busy && !on ? 0.5 : 1)
              .bg(on ? t.emph : t.bg).text_color(on ? t["on-emph"] : hint ? t["action-text"] : t.text)
              .text_size(S["type-small"][0]).font_weight(on || hint ? 600 : 500)
              .when(!busy, (el) => el.hover((s) => s.bg(t.raised))).focus((s) => s.border(2).border_color(t["action-text"]))
              .accessibility_label([option.label, option.recommended && "recommended", option.key && `key ${option.key}`].filter(Boolean).join(", "))
              .on_click((_event, cx) => this.choose(model, row, option.label, cx))
              .when(hint, (el) => el.child(svg(ICON("check")).size(13)))
              .child(option.label);
          })))
        .when(Boolean(pick.note) && !pending, (el) => el.child(div().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(pick.note ?? "")))
        .when(commentable && !pending, (el) => el.child(this.secondary(`comment-${row.id}`, "Comment", (cx) => this.select(model, row.id, cx), t)))
        .when(reopened && !pending, (el) => el.child(this.secondary("change-cancel", "Keep", (cx) => { this.changing = null; cx.notify(); }, t)))
        .when(Boolean(pending), (el) => el.child(h_flex().flex_1().gap(S["space-2"]).items_center().justify_end().text_size(S["type-small"][0])
          .child(div().child(pending?.timer ? `Picking ${pending.option}` : `Sending ${pending?.option}…`))
          .when(Boolean(pending?.timer), (el) => el.child(this.secondary("undo", "Undo", (cx) => this.undo(cx), t))))));
  }

  /** A row's threads under it, a resolved one folded to one line, then the comment field on the selected row. */
  /** @param {Dossier} model @param {Row} row @param {Palette} t */
  threads(model, row, t) {
    const selected = row.id === this.selected && (this.threadable(row) || this.replyTo != null);
    const notice = row.pick ? undefined : this.notices.get(row.id);
    return v_flex().pl(S["pane-tile"] + 10 + S["space-2"]).pt(S["space-1"]).pb(S["space-2"]).gap(S["space-2"])
      .children(row.threads.map((thread) => {
        if (thread.resolved) {
          return h_flex().gap(S["space-2"]).items_center().text_size(S["type-caption"][0]).text_color(t["text-3"])
            .child(svg(ICON("check")).size(13).flex_none().text_color(t["ok-text"]))
            .child(div().flex_1().min_w_0().truncate().child(`Resolved: ${thread.text}`))
            .child(this.secondary(`reopen-${thread.id}`, "Reopen", (cx) => this.remark(row, (client) => client.reopen(thread.id), cx), t));
        }
        return v_flex().p(S["space-2"]).gap(S["space-2"]).rounded(S["radius-md"]).bg(t.surface)
          .children([thread, ...thread.replies].map((comment) => v_flex().gap(2)
            .child(h_flex().gap(S["space-2"]).text_size(S["type-caption"][0])
              .child(div().font_weight(600).text_color(t["text-2"]).child(comment.author))
              .child(div().text_color(t["text-3"]).child(comment.when ?? "")))
            .child(div().whitespace_normal().text_size(S["type-small"][0]).child(comment.text))))
          .child(h_flex().gap(S["space-2"])
            .child(this.secondary(`reply-${thread.id}`, "Reply", (cx) => {
              this.select(model, row.id, cx);
              this.replyTo = thread.id;
            }, t))
            .child(this.secondary(`resolve-${thread.id}`, "Resolve", (cx) => this.remark(row, (client) => client.resolve(thread.id), cx), t)));
      }))
      .when(Boolean(notice), (el) => el.child(this.notice(notice, t)))
      .when(selected, (el) => el.child(v_flex().gap(S["space-1"])
        .when(this.replyTo != null, (el) => el.child(div().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(`Replying in thread ${this.replyTo}`)))
        .child(h_flex().gap(S["space-2"]).items_center()
          .child(div().flex_1().min_w_0().h(S["control-fine"]).px(8).rounded(S["radius-sm"]).border(1).border_color(t["control-line"])
            .child(Input.new(this.draft).h_full().w_full().text_size(S["type-small"][0]).text_color(t.text)))
          .child(this.secondary("comment-send", this.replyTo == null ? "Comment" : "Reply", (cx) => this.post(cx), t))
          .when(this.replyTo != null, (el) => el.child(this.secondary("comment-cancel", "Cancel", (cx) => { this.replyTo = null; cx.notify(); }, t))))));
  }

  /** @param {string} id @param {string} label @param {(cx: import("gpui-kit").Context) => void} action @param {Palette} t */
  secondary(id, label, action, t) {
    return Button.new(id).h(S["control-fine"] - 4).px(S["space-3"]).flex().flex_none().items_center().rounded(S["radius-card"]).border(1).border_color(t["control-line"])
      .bg(t.surface).text_color(t.text).text_size(S["type-small"][0]).font_weight(600).hover((s) => s.bg(t.raised))
      .focus((s) => s.border(2).border_color(t["action-text"])).on_click((_event, cx) => action(cx)).child(label);
  }
}
