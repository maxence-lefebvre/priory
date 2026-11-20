import { View, div, svg } from "gpui-kit";
import { h_flex, v_flex, Button, Input, InputState, Popover, TextView } from "gpui-base";
import { appHome, exec, home as herdrHome, readTextFile, setShared } from "herdr";
import { priory, spansHtml } from "./priory.js";
import { addChoices } from "./add.js";
import { AddForm } from "./add-form.js";
import { FONTS, palette } from "./tokens.js";
import { moveTargets, queueList, reorder, upNext } from "./view.js";

// The --pane-* tokens of design/tokens/structure.css, and the type and radii this panel uses.
const PANE = { split: 720, head: 44, chip: 28, row: 52, tile: 24, control: 32, grip: 20 };
const RADIUS = { sm: 5, tile: 8, md: 12, card: 14 };
// Size and line height; gpui-kit reads a bare line height as a multiplier, so heights are lengths.
/** @type {Record<"title" | "body" | "small" | "caption", [number, `${number}px`]>} */
const TYPE = { title: [18, "24px"], body: [14, "20px"], small: [13, "18px"], caption: [12, "16px"] };
const RETRY_MS = 3000;
const ICON = (/** @type {string} */ name) => `mods/priory-queue/icons/${name}.svg`;
/** A state's tile: its role's fill and its icon (DESIGN.md, State tile). A running tile holds a live dot. */
/** @type {Record<string, [string, string]>} */
const TILES = {
  queued: ["tile-muted", "circle-dashed"],
  ready: ["tile-live", "circle-dot"],
  running: ["tile-live", ""],
  held: ["tile-hold", "pause"],
  done: ["tile-ok", "check"],
  failed: ["tile-danger", "circle-x"],
  dropped: ["tile-muted", "minus"],
};
const WAITING = ["queued", "ready"];
/** @type {Record<string, number>} */
const SHIFTED = { "shift-j": 1, J: 1, "shift-k": -1, K: -1 };

/** @typedef {Record<string, import("gpui-kit").Color>} Palette */
/** @typedef {[string, string, (cx: import("gpui-kit").Context) => void, string?]} MenuItem a label, a key hint, an action and an icon */
/** @typedef {ReturnType<typeof queueList>["sections"][number]["rows"][number] & Record<string, any>} Row */

// settings.json's "priory" object: { binary, machine, factory, theme }, each optional. Mirrors priory-asks.
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
  return { theme: config.theme ?? "indigo", client, links: (await client.links()).links };
}

/** Text with its autolinks; plain text stays a plain child. @param {string} id @param {{ text: string, url?: string }[]} spans */
const linked = (id, spans) => (spans.some((span) => span.url) ? TextView.html(id, spansHtml(spans)) : spans.map((span) => span.text).join(""));

const message = (/** @type {unknown} */ error) => (error instanceof Error ? error.message : String(error));

export default class PrioryQueue extends View {
  /** @param {unknown} _props @param {import("gpui-kit").AsyncContext} cx */
  init(_props, cx) {
    /** @type {{ theme: string, client: ReturnType<typeof priory> } | null} */
    this.priory = null;
    /** @type {any} `priory queue --json` */
    this.data = null;
    /** @type {{ pattern: string, url: string }[]} */
    this.links = [];
    /** @type {number | null} the revision of the read the last frame drew */
    this.drawn = null;
    /** @type {import("./view.js").Room} */
    this.room = {};
    /** @type {string | null} */
    this.error = null;
    this.updated = 0;
    this.width = 420;
    this.view = "next";
    /** @type {{ lane?: string, band?: string }} */
    this.facets = {};
    /** @type {number | null} */
    this.selected = null;
    /** Below 720 px, whether the selected row is open in place. */
    this.opened = false;
    /** @type {string | null} which popover is open: "more", "lane", "band", "move", "hold" or "cancel" */
    this.popover = null;
    /** Whether a verb is in flight; nothing else starts until it ends. */
    this.acting = false;
    /** @type {{ task: number | null, text: string } | null} why the last verb did not go */
    this.notice = null;
    /** @type {number | null} the task whose grip is held */
    this.drag = null;
    /** Counts reads, so a slower, older read never replaces a newer one. */
    this.reads = 0;
    /** The first read that shows the last action's result; the actions stay disabled until it lands. */
    this.settles = 0;
    /** Whether Last day is unfolded under Next. */
    this.lastDay = false;
    this.reason = InputState.new({ placeholder: "Why hold it?" });
    this.reason.on("change", (_event, cx) => cx.notify());
    this.reason.on("submit", (_event, cx) => this.hold(cx));
    /** @type {string | null} the key `priory add` returned, opened once a read lists it */
    this.addedKey = null;
    this.adding = new AddForm(async () => (this.priory ??= await connect()).client, (key) => {
      [this.view, this.facets, this.addedKey] = ["next", {}, key];
    });
    this.focus = cx.focus_handle();
    this.font = FONTS.indigo;
    this.refresh(cx);
  }

  /** Reads the queue and the room, then waits for the next revision and reads again; keeps the last good read on failure. */
  /** @param {import("gpui-kit").AsyncContext} cx */
  refresh(cx) {
    cx.spawn(async (cx) => {
      for (;;) {
        try {
          this.priory ??= await connect();
          this.links = this.priory.links;
          if (await this.read()) this.error = null;
          cx.notify();
          await this.priory.client.wait(this.data.revision);
        } catch (error) {
          this.priory = null;
          this.error = message(error);
          cx.notify();
          await cx.sleep(RETRY_MS);
        }
      }
    });
  }

  /** Reads the queue and the room; false when a newer read started meanwhile and this one was dropped. */
  async read() {
    if (!this.priory) throw new Error("priory is not reachable");
    const read = ++this.reads;
    // The desk carries whether starts are paused and what the queue is short of.
    const [queue, desk] = await Promise.all([this.priory.client.queue(), this.priory.client.desk()]);
    if (read !== this.reads) return false;
    if (read >= this.settles) this.acting = false;
    this.data = queue;
    this.room = { paused: desk.paused, banner: desk.banner };
    this.updated = Date.now();
    const added = this.addedKey && this.rows().find((row) => row.key === this.addedKey);
    if (added) [this.selected, this.opened, this.popover, this.addedKey] = [added.id, true, null, null];
    if (!this.rows().some((row) => row.id === this.selected)) {
      this.selected = this.rows()[0]?.id ?? null;
      this.opened = false;
      this.popover = null;
    }
    return true;
  }

  list(now = Date.now()) {
    return queueList(this.data ?? { waiting: [], running: [], ended: [], up_next: [] }, { view: this.view, now, room: this.room, links: this.links, facets: this.facets });
  }

  /** @returns {Row[]} */
  rows() {
    return this.list().sections.filter((section) => !this.folded(section.id)).flatMap((section) => section.rows);
  }

  selectedRow() {
    return this.rows().find((row) => row.id === this.selected) ?? null;
  }

  /** @param {number | null} id @param {import("gpui-kit").Context} cx */
  select(id, cx) {
    this.focus.focus();
    this.opened = id !== null && id === this.selected ? !this.opened : id !== null;
    this.selected = id;
    this.popover = null;
    cx.notify();
  }

  /** After a view or facet changes, its first row is selected. @param {import("gpui-kit").Context} cx */
  refilter(cx) {
    this.popover = null;
    this.selected = this.rows()[0]?.id ?? null;
    this.opened = false;
    cx.notify();
  }

  /**
   * Runs one verb on the revision the panel drew, then reads the queue again. A verb that fails, or
   * that priory refuses because the state moved on, says why on its task.
   * @param {number | null} task @param {string} label @param {(client: ReturnType<typeof priory>, revision: number | null) => Promise<unknown>} verb @param {import("gpui-kit").Context} cx
   */
  act(task, label, verb, cx) {
    this.popover = null;
    if (this.acting || this.error) return cx.notify();
    const revision = this.drawn;
    this.acting = true;
    this.settles = Infinity;
    this.notice = null;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        if (!this.priory) throw new Error("priory is not reachable");
        await verb(this.priory.client, revision);
      } catch (error) {
        this.notice = { task, text: `${label} did not go: ${message(error)}.` };
      }
      this.settles = this.reads + 1;
      try {
        await this.read();
      } catch (error) {
        // The actions stay disabled until the refresh loop applies a read taken after the action.
        this.error = message(error);
      }
      cx.notify();
    });
  }

  /** @param {number} task @param {number | { onto: number }} to @param {import("gpui-kit").Context} cx */
  reorder(task, to, cx) {
    const place = this.data && reorder(this.data, task, to);
    if (place) this.act(task, "Move", (client, revision) => client.move(task, place, revision), cx);
  }

  /** @param {import("gpui-kit").Context} cx */
  hold(cx) {
    const reason = this.reason.value().trim();
    const task = this.selected;
    if (!reason || task === null) return;
    this.reason.set_value("");
    this.act(task, "Hold", (client, revision) => client.hold(task, { reason }, revision), cx);
  }

  /** @param {import("gpui-kit").KeyEvent} event @param {import("gpui-kit").Context} cx */
  key(event, cx) {
    if (event.key === "escape") {
      if (this.adding.open) {
        this.adding.hide(cx);
        this.focus.focus();
      }
      else if (this.popover) this.popover = null;
      else this.opened = false;
      return cx.notify();
    }
    // The hold reason field keeps its letters, and Ctrl, Alt and Cmd chords are Herdr's.
    if (this.popover || this.adding.typing || event.modifiers?.control || event.modifiers?.platform || event.modifiers?.alt) return;
    if (event.key === "n") return this.adding.show(cx);
    const row = this.selectedRow();
    // Shift J and K arrive as the keystroke "shift-j" or as the key "J", as Fleet's queue sees them.
    const shifted = SHIFTED[event.keystroke] ?? SHIFTED[event.key];
    if (shifted) {
      if (row && !event.is_held) this.reorder(row.id, shifted, cx);
      return;
    }
    if (event.key === "j" || event.key === "down" || event.key === "k" || event.key === "up") {
      const ids = this.rows().map((candidate) => candidate.id);
      const step = event.key === "j" || event.key === "down" ? 1 : -1;
      const next = ids[Math.min(ids.length - 1, Math.max(0, ids.indexOf(/** @type {number} */ (this.selected)) + step))];
      if (next === undefined) return;
      this.selected = next;
      this.opened = false;
      return cx.notify();
    }
    if (event.key === "enter" && row && !event.is_held) {
      this.opened = !this.opened;
      cx.notify();
    }
  }

  /** @param {import("gpui-kit").Context} cx */
  render(cx) {
    // Each action sends the revision of the read on screen, so priory refuses it once the queue changed.
    this.drawn = this.data?.revision ?? null;
    const theme = this.priory?.theme ?? "indigo";
    const t = /** @type {Palette} */ (palette(theme, cx.theme().appearance));
    this.font = FONTS[/** @type {keyof typeof FONTS} */ (theme)] ?? FONTS.indigo;
    const now = Date.now();
    const list = this.list(now);
    const row = this.selectedRow();
    const wide = this.width >= PANE.split;
    return v_flex().id("priory-queue").size_full().min_h_0().bg(t.bg).text_color(t.text).font_family(this.font.ui).text_size(TYPE.body[0])
      .track_focus(this.focus).on_key_down((event, cx) => this.key(event, cx))
      .on_mouse_up("left", (_event, cx) => this.endDrag(cx))
      // A release outside the pane never reaches it, so leaving the pane ends the drag without a move.
      .on_hover((hovered, cx) => { if (!hovered) this.endDrag(cx); })
      .on_size_change(({ width }, cx) => {
        if (Math.abs(width - this.width) < 1) return;
        this.width = width;
        cx.notify();
      })
      .child(this.header(list, t))
      .when(Boolean(this.error), (el) => el.child(this.strip(`Reconnecting to priory: ${this.error}.${this.age(now)}`, t)))
      .when(Boolean(list.paused), (el) => el.child(this.strip(list.paused ?? "", t)))
      .when(Boolean(this.notice) && !this.rows().some((candidate) => candidate.id === this.notice?.task), (el) => el.child(this.strip(this.notice?.text ?? "", t)))
      .when(this.adding.open, (el) => el.child(div().flex_none().px(16).pt(8).child(this.adding.render(t, "mods/priory-queue/icons", addChoices(this.data ?? { waiting: [] }, [])))))
      .child(this.filters(list, t))
      .child(wide ? h_flex().flex_1().min_h_0()
        .child(this.listPane(list, t, now, false).w(Math.min(400, this.width * 0.45)).flex_none().border_r(1).border_color(t.line))
        .child(row ? v_flex().id("queue-detail").flex_1().min_w_0().min_h_0().overflow_y_scrollbar().p(16).gap(12).child(this.detail(row, t, true)) : div().flex_1())
        : this.listPane(list, t, now, true).flex_1());
  }

  /** @param {import("gpui-kit").Context} cx */
  endDrag(cx) {
    if (this.drag === null) return;
    this.drag = null;
    cx.notify();
  }

  /** @param {number} now */
  age(now) {
    if (!this.data) return "";
    const minutes = Math.round((now - this.updated) / 60_000);
    return ` Last update ${minutes < 1 ? "under a minute" : `${minutes} min`} ago.`;
  }

  /** @param {string} text @param {Palette} t */
  strip(text, t) {
    return div().flex_none().px(16).py(6).whitespace_normal().text_size(TYPE.small[0]).text_color(t["warn-text"]).border_b(1).border_color(t.line).child(text);
  }

  /** @param {ReturnType<typeof queueList>} list @param {Palette} t */
  header(list, t) {
    const open = this.popover === "more";
    const paused = Boolean(this.room.paused);
    const trigger = Button.new("queue-more").size(PANE.control).flex().flex_none().items_center().justify_center().rounded(RADIUS.sm)
      .selected(open).bg(open ? t.raised : t.bg).hover((s) => s.bg(t.raised)).focus((s) => s.bg(t.raised))
      .accessibility_label("More for the queue").child(svg(ICON("more-horizontal")).size(16).text_color(t["text-2"]));
    return h_flex().flex_none().h(PANE.head).px(16).gap(8).items_center().border_b(1).border_color(t.line)
      .child(div().font_family(this.font.display).font_weight(700).text_size(16).child("Queue"))
      .child(div().flex_1().text_size(TYPE.small[0]).text_color(t["text-3"]).child(`${list.waiting} waiting`))
      .child(Button.new("add-open").h(PANE.control).px(8).gap(4).flex().flex_none().items_center().rounded(RADIUS.sm)
        .bg(t.bg).hover((s) => s.bg(t.raised)).focus((s) => s.bg(t.raised)).text_size(TYPE.small[0]).text_color(t.text)
        .accessibility_label("Add a task, N").on_click((_event, cx) => this.adding.show(cx))
        .child(svg(ICON("plus")).size(16).text_color(t["text-2"])).child("Add"))
      .child(Popover.new("queue-more-menu").trigger(trigger).anchor("top_right").open(open)
        .on_open_change((/** @type {boolean} */ next, /** @type {import("gpui-kit").Context} */ cx) => { this.popover = next ? "more" : null; cx.notify(); })
        .content(this.menu(t, [paused
          ? ["Unpause starts", "", (cx) => this.act(null, "Unpause", (client) => client.unpause(), cx), "play"]
          : ["Pause starts", "", (cx) => this.act(null, "Pause", (client) => client.pause(), cx), "pause"]])));
  }

  /** @param {ReturnType<typeof queueList>} list @param {Palette} t */
  filters(list, t) {
    /** @param {"lane" | "band"} facet @param {string} label */
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
      .child(h_flex().gap(6).flex_wrap().children(list.views.map((view) => {
        const on = this.view === view.id;
        return Button.new(`view-${view.id}`).h(PANE.chip).px(10).flex().flex_none().items_center().gap(4).rounded(999)
          .selected(on).text_size(TYPE.small[0]).font_weight(on ? 600 : 500).bg(on ? t.emph : t.surface).text_color(on ? t["on-emph"] : t.text)
          .hover((s) => s.opacity(0.88)).focus((s) => s.border(2).border_color(t["action-text"]))
          .on_click((_event, cx) => { this.view = view.id; this.refilter(cx); })
          .child(view.label).child(div().opacity(0.7).child(view.id === "all" ? "" : String(view.count)));
      })))
      .child(h_flex().gap(6).flex_wrap().child(picker("lane", "Lane")).child(picker("band", "Band")));
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

  /** @param {ReturnType<typeof queueList>} list @param {Palette} t @param {number} now @param {boolean} inPlace */
  listPane(list, t, now, inPlace) {
    const empty = {
      next: "Nothing waits or runs. Requested tasks line up here by band.",
      held: "No held tasks. A task held by a person or a condition waits here with why.",
      ended: "No task ended in the last day.",
      all: "No tasks yet.",
    }[this.view] ?? "";
    if (!this.data) return v_flex().id("queue-list").p(16).text_color(t["text-3"]).child(this.error ? "" : "Reading the queue…");
    return v_flex().id("queue-list").min_h_0().overflow_y_scrollbar().pb(16)
      .when(this.view === "next" && this.data.up_next.length > 0, (el) => el.child(this.upNext(t, now)))
      .when(!list.sections.length, (el) => el.child(div().p(16).text_color(t["text-2"]).child(empty)))
      .children(list.sections.map((section) => v_flex()
        .child(this.sectionHead(section, t))
        .children((this.folded(section.id) ? [] : section.rows).map((/** @type {Row} */ row) => {
          const selected = row.id === this.selected;
          return v_flex().px(8)
            .child(this.row(row, selected, t))
            .when(inPlace && selected && this.opened, (el) => el.child(v_flex().px(8).pb(12).gap(10).child(this.detail(row, t, false))));
        }))));
  }

  /** Last day starts folded under Next. @param {string} id */
  folded(id) {
    return id === "ended" && this.view === "next" && !this.lastDay;
  }

  /** @param {{ id: string, title: string, note: string }} section @param {Palette} t */
  sectionHead(section, t) {
    const head = h_flex().px(16).pt(14).pb(6).gap(6).items_center().text_size(TYPE.caption[0]).font_weight(600).text_color(t["text-2"])
      .child(section.title).child(div().font_weight(400).text_color(t["text-3"]).child(section.note));
    if (section.id !== "ended" || this.view !== "next") return head;
    return Button.new("fold-ended").w_full().focus((s) => s.bg(t.raised)).selected(this.lastDay)
      .accessibility_label(this.lastDay ? "Fold Last day" : "Unfold Last day")
      .on_click((_event, cx) => { this.lastDay = !this.lastDay; cx.notify(); })
      .child(head.child(svg(ICON(this.lastDay ? "chevron-down" : "chevron-right")).size(12).text_color(t["text-3"])));
  }

  /** The next three runs, queued or scheduled, each opening its task. @param {Palette} t @param {number} now */
  upNext(t, now) {
    // A schedule that wakes an asleep task names a task the queue does not list.
    const all = queueList(this.data, { view: "all", now }).sections.flatMap((section) => section.rows);
    const listed = (/** @type {number | null} */ task) => all.some((row) => row.id === task);
    return v_flex().mx(16).mt(12).p(4).rounded(RADIUS.md).border(1).border_color(t.line).bg(t.surface)
      .child(h_flex().px(8).py(4).gap(6).items_center().text_size(TYPE.caption[0])
        .child(svg(ICON("calendar-clock")).size(14).text_color(t["text-2"]))
        .child(div().font_weight(600).child("Up next")).child(div().text_color(t["text-3"]).child("the next 3 runs")))
      .children(upNext(this.data, now).map((next, index) => Button.new(`next-${index}`).px(8).py(6).flex().gap(10).items_start().rounded(RADIUS.sm)
        .hover((s) => s.bg(t.raised)).focus((s) => s.bg(t.raised)).disabled(!listed(next.task))
        .accessibility_label(`${next.when} ${next.sub}: ${next.label}${next.title ? `, ${next.title}` : ""}. ${next.why}`)
        .on_click((_event, cx) => {
          if (!listed(next.task)) return;
          if (!this.rows().some((row) => row.id === next.task)) {
            this.view = "all";
            this.facets = {};
          }
          this.selected = null;
          this.lastDay = true;
          this.select(next.task, cx);
        })
        .child(v_flex().w(48).flex_none()
          .child(div().font_family(this.font.mono).font_weight(600).text_size(TYPE.small[0]).child(next.when))
          .child(div().text_size(TYPE.caption[0]).text_color(t["text-3"]).child(next.sub)))
        .child(v_flex().flex_1().min_w_0()
          .child(div().truncate().font_family(this.font.mono).text_size(TYPE.small[0]).child(next.label))
          .when(Boolean(next.title), (el) => el.child(div().truncate().font_weight(600).text_size(TYPE.small[0]).child(next.title ?? "")))
          .child(div().whitespace_normal().text_size(TYPE.caption[0]).text_color(t["text-2"]).child(next.why)))));
  }

  /** @param {Row} row @param {boolean} selected @param {Palette} t */
  row(row, selected, t) {
    const reorderable = WAITING.includes(row.tile);
    return h_flex().w_full().items_start().rounded(RADIUS.md).bg(selected ? t["action-soft"] : t.bg)
      .when(this.drag !== null && reorderable, (el) => el.on_mouse_up("left", (_event, cx) => {
        const dragged = this.drag;
        this.drag = null;
        if (dragged !== null) this.reorder(dragged, { onto: row.id }, cx);
        cx.stop_propagation();
      }))
      .child(Button.new(`row-${row.id}`).flex_1().min_w_0().min_h(PANE.row).px(8).py(8).flex().items_start().gap(10).rounded(RADIUS.md)
        .selected(selected).hover((s) => s.bg(selected ? t["action-soft"] : t.raised)).focus((s) => s.border(2).border_color(t["action-text"]))
        .accessibility_label(`${row.tile} ${row.key}${row.title ? `, ${row.title}` : ""}: ${row.why?.text ?? row.step}`)
        .on_click((_event, cx) => this.select(row.id, cx))
        .child(this.tile(row.tile, t))
        .child(v_flex().flex_1().min_w_0().gap(2)
          .child(h_flex().gap(6).items_center().text_size(TYPE.caption[0]).text_color(t["text-3"])
            .when(row.position != null, (el) => el.child(div().font_weight(600).text_color(t["text-2"]).child(String(row.position))))
            .child(div().font_family(this.font.mono).text_color(t.text).child(row.key))
            .child(div().child(row.step))
            .child(div().text_color(row.tile === "running" ? t["action-text"] : t["text-3"]).child(row.tile))
            .child(div().flex_1())
            .when(Boolean(row.age), (el) => el.child(div().whitespace_nowrap().child(row.age))))
          .when(Boolean(row.title), (el) => el.child(div().truncate().font_weight(600).text_size(TYPE.body[0]).line_height(TYPE.body[1]).child(row.title ?? "")))
          .when(Boolean(row.why), (el) => el.child(div().whitespace_normal().text_size(TYPE.small[0]).line_height(TYPE.small[1]).text_color(this.tone(row.why.tone, t)).child(row.why.text)))
          .when(Boolean(row.conditions?.length), (el) => el.children(row.conditions.map((/** @type {{ text: string, met: boolean }} */ item) => this.condition(item, t))))))
      .child(reorderable
        ? div().id(`grip-${row.id}`).w(PANE.grip).flex_none().flex().justify_center().pt(10).cursor_grab()
          .on_mouse_down("left", (_event, cx) => { this.drag = row.id; this.selected = row.id; cx.stop_propagation(); cx.notify(); })
          .child(svg(ICON("grip-vertical")).size(14).text_color(t["text-3"]))
        : div().w(PANE.grip).flex_none());
  }

  /** @param {string} tone @param {Palette} t */
  tone(tone, t) {
    return tone === "warn" ? t["warn-text"] : tone === "held" ? t["text-2"] : t["text-3"];
  }

  /** @param {{ text: string, met: boolean }} item @param {Palette} t */
  condition(item, t) {
    return h_flex().gap(4).items_center().text_size(TYPE.caption[0]).text_color(item.met ? t["ok-text"] : t["text-2"])
      .child(svg(ICON(item.met ? "check" : "clock")).size(12).flex_none()).child(div().whitespace_normal().child(item.text));
  }

  /** @param {string} state @param {Palette} t */
  tile(state, t) {
    const [fill, icon] = TILES[state] ?? ["tile-muted", "circle-help"];
    return h_flex().size(PANE.tile).flex_none().items_center().justify_center().rounded(RADIUS.tile).bg(t[fill])
      .accessibility_label(state)
      .child(icon ? svg(ICON(icon)).size(14).text_color(t["on-tile"]) : div().size(8).rounded(999).bg(t["on-tile-live"]));
  }

  /** What an opened row shows: why it waits, and the person's actions. @param {Row} row @param {Palette} t @param {boolean} wide */
  detail(row, t, wide) {
    const notice = this.notice?.task === row.id ? this.notice.text : null;
    return v_flex().gap(12)
      .when(wide, (el) => el
        .child(h_flex().flex_wrap().gap(6).items_center().text_size(TYPE.caption[0]).text_color(t["text-3"])
          .child(this.tile(row.tile, t)).child(div().font_family(this.font.mono).text_color(t["text-2"]).child(row.key)).child(row.step).child(row.tile))
        .child(div().whitespace_normal().font_family(this.font.display).font_weight(700).text_size(TYPE.title[0]).line_height(TYPE.title[1]).child(row.title ? linked(`title-${row.id}`, row.titleSpans) : row.key)))
      .when(Boolean(row.why) && wide, (el) => el.child(v_flex().p(12).gap(6).rounded(RADIUS.md).border(1).border_color(row.why.tone === "warn" ? t["warn-text"] : t.line)
        .child(div().whitespace_normal().child(row.why.text))
        .children((row.conditions ?? []).map((/** @type {{ text: string, met: boolean }} */ item) => this.condition(item, t)))))
      .when(Boolean(notice), (el) => el.child(div().whitespace_normal().text_size(TYPE.small[0]).text_color(t["warn-text"]).child(notice ?? "")))
      .child(this.actions(row, t))
      .when(wide && WAITING.includes(row.tile), (el) => el.child(v_flex().gap(6)
        .child(div().text_size(TYPE.small[0]).font_weight(650).child("Order"))
        .child(div().whitespace_normal().text_size(TYPE.small[0]).text_color(t["text-2"])
          .child("A higher band starts first. Inside a band, the order you set with the grip or Shift J and Shift K wins, then the oldest first."))));
  }

  /** Move…, Hold… or Release, Cancel…, and Attach for a live run. @param {Row} row @param {Palette} t */
  actions(row, t) {
    const off = this.acting || Boolean(this.error);
    /** @param {string} id @param {string} icon @param {string} label @param {(cx: import("gpui-kit").Context) => void} action */
    const button = (id, icon, label, action) => Button.new(`${id}-${row.id}`).h(PANE.control).px(10).flex().flex_none().items_center().gap(6)
      .rounded(RADIUS.card).border(1).border_color(t["control-line"]).bg(this.popover === id ? t.raised : t.surface).text_color(t.text)
      .text_size(TYPE.small[0]).font_weight(600).disabled(off).opacity(off ? 0.5 : 1).hover((s) => s.bg(t.raised))
      .focus((s) => s.border(2).border_color(t["action-text"])).on_click((_event, cx) => action(cx))
      .when(Boolean(icon), (el) => el.child(svg(ICON(icon)).size(14))).child(label);
    /** @param {string} id @param {string} icon @param {string} label @param {any} content */
    const popover = (id, icon, label, content) => Popover.new(`${id}-menu-${row.id}`).anchor("top_left").open(this.popover === id && !off)
      .trigger(button(id, icon, label, () => {}))
      .on_open_change((/** @type {boolean} */ next, /** @type {import("gpui-kit").Context} */ cx) => {
        // A hold reason belongs to the task it was typed for.
        if (next && id === "hold") this.reason.set_value("");
        this.popover = next ? id : null;
        cx.notify();
      })
      .content(content);
    const task = row.id;
    const waiting = WAITING.includes(row.tile);
    const targets = this.data ? moveTargets(this.data, task) : [];
    return h_flex().gap(6).flex_wrap().items_center()
      .when(targets.length > 0, (el) => el.child(popover("move", "arrow-up-to-line", "Move…", this.menu(t, targets.map(({ label, move }) =>
        [label, "", (cx) => this.act(task, "Move", (client, revision) => client.move(task, move, revision), cx)])))))
      .when(waiting, (el) => el.child(popover("hold", "pause", "Hold…", v_flex().w(260).p(10).gap(8).rounded(RADIUS.md).bg(t.surface).border(1).border_color(t["control-line"])
        .child(div().text_size(TYPE.small[0]).font_weight(600).child(`Hold ${row.key} until a person releases it`))
        .child(div().h(PANE.control).px(8).rounded(RADIUS.sm).border(1).border_color(t["control-line"]).child(Input.new(this.reason).h_full().w_full().text_size(TYPE.small[0]).text_color(t.text)))
        .child(button("hold-confirm", "", "Hold", (cx) => this.hold(cx))))))
      .when(row.tile === "held", (el) => el.child(button("release", "play", "Release", (cx) => this.act(task, "Release", (client, revision) => client.release(task, revision), cx))))
      .when(row.tile === "running", (el) => el.child(button("attach", "terminal", "Attach", (cx) => {
        setShared("priory.attach", { task, at: Date.now() });
        cx.notify();
      })))
      .when(waiting || row.tile === "held" || row.tile === "running", (el) => el.child(popover("cancel", "x", "Cancel…", v_flex().w(260).p(10).gap(8).rounded(RADIUS.md).bg(t.surface).border(1).border_color(t["control-line"])
        .child(div().whitespace_normal().text_size(TYPE.small[0]).child(`Cancel ${row.key}? Its run stops and the task ends as dropped.`))
        .child(Button.new(`cancel-confirm-${task}`).h(PANE.control).px(12).flex().items_center().rounded(RADIUS.card).bg(t["tile-danger"]).text_color(t["on-tile"])
          .font_weight(600).focus((s) => s.border(2).border_color(t["action-text"]))
          .on_click((_event, cx) => this.act(task, "Cancel", (client, revision) => client.cancel(task, revision), cx)).child(`Cancel ${row.key}`)))));
  }
}
