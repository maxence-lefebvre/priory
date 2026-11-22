import { Background, PathBuilder, View, div, svg } from "gpui-kit";
import { h_flex, v_flex, Button } from "gpui-base";
import { appHome, exec, home as herdrHome, readTextFile } from "herdr";
import { priory } from "./priory.js";
import { selectable } from "./ui.js";
import { FONTS, palette, STRUCTURE as S } from "./tokens.js";
import { attentionView, costTiles, dayBars, money, scoreTable, splitTable, SPLITS } from "./view.js";

// From this width Costs and Attention sit side by side (DESIGN.md, Costs panel).
const SIDE_BY_SIDE = 1024;
// From this width each day's bar names its weekday.
const WEEKDAYS = 520;
// Below this width each score row stacks, as the web's does below 600 px.
const SCORE_COLUMNS = 600;
const CHART = 96;
const RETRY_MS = 3000;
const READ_EVERY_MS = 60_000;
const ICON = (/** @type {string} */ name) => `mods/priory-costs/icons/${name}.svg`;
const TRENDS = { up: "trending-up", down: "trending-down" };
const PARTS = [["working", "Working"], ["person", "Waiting on you"], ["world", "Waiting on the world"], ["queued", "Queued"]];
const VIEWS = [["costs", "Costs"], ["attention", "Attention"]];
// A rectangle over the whole element, for a fill gpui-kit gives only as a Background.
const WHOLE = PathBuilder.fill().add_polygon([[0, 0], ["100%", 0], ["100%", "100%"], [0, "100%"]]).build();

/** @typedef {Record<string, import("gpui-kit").Color>} Palette */
/** @typedef {typeof FONTS.indigo} Fonts */
/** @typedef {{ costs: any, attention: any }} Read */
/** @typedef {{ label: string, value: string, delta?: { text: string, direction: string } | null, note?: string }} Tile */
/** @typedef {{ weekday: string, date: string, fraction: number, merges: number, current: boolean, text: string }} Bar */
/** @typedef {{ dimension: string, rows: { name: string, cost: string, tokens: string, note: string }[], unpriced: { model: string, tokens: string }[], unknown: string | null }} Table */
/** @typedef {{ dimension: string, rows: { name: string, scorer: string, rate: string, pass: number, fail: number, error: number, notApplicable: number, line: string }[] }} Scores */
/** @typedef {{ part: string, label: string, fraction: number, value: string }} Part */
/** @typedef {{ key: string, workflow: string, wall: string, text: string, parts: Part[] }} TaskTime */

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
const readAt = (/** @type {number} */ at) => new Date(at).toLocaleString("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23" });

export default class PrioryCosts extends View {
  /** @param {unknown} _props @param {import("gpui-kit").AsyncContext} cx */
  init(_props, cx) {
    /** @type {Awaited<ReturnType<typeof connect>> | null} */
    this.priory = null;
    /** @type {Read | null} */
    this.data = null;
    /** @type {string | null} */
    this.error = null;
    this.updated = 0;
    this.width = 420;
    this.view = "costs";
    /** @type {string} the split the table shows */
    this.by = "workflow";
    /** Bumped by each read, so a read started before another split was picked is dropped. */
    this.reads = 0;
    /** @type {Fonts} */
    this.font = FONTS.indigo;
    this.refresh(cx);
  }

  // The reports have no revision to wait on, and their window ends now, so they are read once a minute.
  /** @param {import("gpui-kit").AsyncContext} cx */
  refresh(cx) {
    cx.spawn(async (cx) => {
      for (;;) {
        try {
          this.priory ??= await connect();
          await this.read();
          cx.notify();
          await cx.sleep(READ_EVERY_MS);
        } catch (error) {
          this.priory = null;
          this.error = message(error);
          cx.notify();
          await cx.sleep(RETRY_MS);
        }
      }
    });
  }

  /** Keeps the last read when a call fails; a read overtaken by a newer one is dropped, failed or not. */
  async read() {
    const client = /** @type {NonNullable<typeof this.priory>} */ (this.priory).client;
    const [read, by] = [++this.reads, this.by];
    /** @type {any[]} */
    let reports;
    try {
      reports = await Promise.all([client.costs(by), client.attention()]);
    } catch (error) {
      if (read === this.reads) throw error;
      return;
    }
    if (read !== this.reads) return;
    const [costs, attention] = reports;
    this.data = { costs, attention };
    this.updated = Date.now();
    this.error = null;
  }

  /** @param {string} by @param {import("gpui-kit").Context} cx */
  split(by, cx) {
    if (by === this.by || !this.priory) return;
    this.by = by;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        await this.read();
      } catch (error) {
        this.error = message(error);
      }
      cx.notify();
    });
  }

  /** @param {import("gpui-kit").Context} cx */
  render(cx) {
    const theme = this.priory?.theme ?? "indigo";
    const t = /** @type {Palette} */ (palette(theme, cx.theme().appearance));
    this.font = FONTS[/** @type {keyof typeof FONTS} */ (theme)] ?? FONTS.indigo;
    const wide = this.width >= SIDE_BY_SIDE;
    const now = Date.now();
    return v_flex().id("priory-costs").size_full().min_h_0().bg(t.bg).text_color(t.text).font_family(this.font.ui).text_size(S["type-body"][0])
      .on_size_change(({ width }, cx) => {
        if (Math.abs(width - this.width) < 1) return;
        this.width = width;
        cx.notify();
      })
      .child(h_flex().flex_none().h(S["pane-head"]).px(S["space-4"]).gap(S["space-2"]).items_center().border_b(1).border_color(t.line)
        .child(div().font_weight(650).child("Costs"))
        .child(div().flex_1())
        .when(!wide, (el) => el.children(VIEWS.map(([id, label]) => this.chip(`view-${id}`, label, this.view === id, (cx) => { this.view = id; cx.notify(); }, t)))))
      .when(Boolean(this.error), (el) => el.child(this.offline(t)))
      .when(Boolean(this.data) && (Boolean(this.error) || now - this.updated > READ_EVERY_MS), (el) => el.child(
        div().flex_none().px(S["space-4"]).py(S["space-2"]).text_size(S["type-caption"][0]).text_color(t["text-3"]).child(`Costs as read ${readAt(this.updated)}.`)))
      .child(this.body(t, wide));
  }

  /** @param {Palette} t */
  offline(t) {
    return h_flex().flex_none().px(S["space-4"]).py(S["space-2"]).gap(S["space-2"]).items_start().text_size(S["type-small"][0]).text_color(t["warn-text"]).border_b(1).border_color(t.line)
      .child(div().flex_1().min_w_0().whitespace_normal().child(selectable("costs-error", `Could not load costs: ${this.error}. Check that priory runs, then it reads again on its own.`)));
  }

  /** @param {Palette} t @param {boolean} wide */
  body(t, wide) {
    const pad = wide ? S["space-5"] : S["space-4"];
    if (!this.data) {
      return v_flex().id("costs-body").flex_1().p(pad).text_color(t["text-2"]).child(this.error ? "" : "Loading costs…");
    }
    const { costs, attention } = this.data;
    const scroll = (/** @type {string} */ id) => v_flex().id(id).flex_1().min_w_0().min_h_0().overflow_y_scrollbar().px(pad).pt(pad).pb(S["space-6"]).gap(S["space-5"]);
    if (!wide) {
      return scroll("costs-body").children(this.view === "costs" ? this.costsPart(costs, t) : this.attentionPart(attention, t));
    }
    return h_flex().flex_1().min_h_0()
      .child(scroll("costs-left").children(this.costsPart(costs, t)))
      .child(div().w(1).h_full().bg(t.line))
      .child(scroll("costs-right").children(this.attentionPart(attention, t)));
  }

  /** @param {string} title @param {string | null} meta @param {Palette} t */
  heading(title, meta, t) {
    return h_flex().gap(S["space-2"]).items_baseline().text_size(S["type-small"][0]).font_weight(650)
      .child(title).when(Boolean(meta), (el) => el.child(div().font_weight(400).text_color(t["text-3"]).child(meta ?? "")));
  }

  /** @param {Tile[]} tiles @param {Palette} t */
  tiles(tiles, t) {
    return h_flex().rounded(S["radius-md"]).border(1).border_color(t.line).bg(t.surface)
      .children(tiles.map((tile, index) => v_flex().flex_1().min_w_0().p(S["space-3"]).gap(S["space-1"]).when(index > 0, (el) => el.border_l(1).border_color(t.line))
        .accessibility_label(`${tile.label}: ${tile.value}`)
        .child(div().truncate().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(tile.label))
        .child(div().font_family(this.font.display).font_weight(650).text_size(S["type-hero"][0]).line_height(`${S["type-hero"][1]}px`).child(tile.value))
        .child(this.tileNote(tile, t))));
  }

  /** A comparison's trend, never colored, since more cost is not good or bad by itself. @param {Tile} tile @param {Palette} t */
  tileNote(tile, t) {
    const line = h_flex().gap(S["space-1"]).items_center().text_size(S["type-caption"][0]).text_color(t["text-2"]);
    if (!("delta" in tile)) return line.child(div().whitespace_normal().child(tile.note ?? ""));
    if (!tile.delta) return line.child("No change to show");
    const icon = TRENDS[/** @type {keyof typeof TRENDS} */ (tile.delta.direction)];
    return line.when(Boolean(icon), (el) => el.child(svg(ICON(icon)).size(13).flex_none())).child(div().truncate().child(tile.delta.text));
  }

  /** @param {any} costs @param {Palette} t */
  costsPart(costs, t) {
    if (!costs.days.length && !costs.split.length) {
      // Scores can land in a window with no cost, so they keep their table and its split chips.
      return [div().whitespace_normal().text_color(t["text-2"]).child("No cost in the last two weeks. Costs appear once a model step records its tokens. Price each model in prices.toml in the factory folder."),
        ...this.scoresPart(costs, t, true)];
    }
    const table = /** @type {Table} */ (splitTable(costs));
    const missing = [
      table.unpriced.length ? `Tokens with no price: ${table.unpriced.map((entry) => `${entry.model} ${entry.tokens} tokens`).join(", ")}. Add each model to prices.toml to price them.` : "",
      table.unknown ? `${table.unknown}, so their cost is unknown.` : "",
    ].filter(Boolean).join(" ");
    return [
      v_flex().gap(S["space-2"]).child(this.heading("This week", "against last week", t)).child(this.tiles(costTiles(costs), t))
        .when(Boolean(missing), (el) => el.child(div().whitespace_normal().text_size(S["type-small"][0]).text_color(t["text-2"]).child(`Not in the cost. ${missing}`))),
      v_flex().gap(S["space-2"]).child(this.heading("Each day", null, t)).child(this.days(costs, t)),
      v_flex().gap(S["space-2"])
        .child(h_flex().gap(S["space-2"]).items_center().flex_wrap()
          .child(this.heading(`This week by ${costs.by}`, null, t)).child(div().flex_1()).children(this.splitChips(t)))
        .child(this.splitRows(table, t)),
      ...this.scoresPart(costs, t, false),
    ];
  }

  /** @param {Palette} t */
  splitChips(t) {
    return SPLITS.map((by) => this.chip(`by-${by}`, by, by === this.by, (cx) => this.split(by, cx), t, Boolean(this.error)));
  }

  /** Each scorer's verdicts by the split, or nothing when the window has no score. @param {any} costs @param {Palette} t @param {boolean} chips */
  scoresPart(costs, t, chips) {
    const table = /** @type {Scores} */ (scoreTable(costs));
    if (!table.rows.length) return [];
    /** @param {string[]} cells @param {boolean} head */
    const row = (cells, head) => h_flex().min_h(head ? 28 : 36).gap(S["space-3"]).items_center().border_b(1).border_color(t.line)
      .text_size(head ? S["type-caption"][0] : S["type-small"][0]).text_color(head ? t["text-3"] : t.text)
      .children(cells.slice(0, 2).map((cell) => div().w(120).flex_none().whitespace_normal().font_family(head ? this.font.ui : this.font.mono).child(cell)))
      .children(cells.slice(2).map((cell, index) => div().w(index ? 56 : 72).flex_none().whitespace_normal().text_right()
        .when(!head && index === 0, (el) => el.font_weight(600)).child(cell)));
    const stacked = (/** @type {Scores["rows"][number]} */ entry) => v_flex().py(S["space-2"]).gap(S["space-1"]).border_b(1).border_color(t.line)
      .child(h_flex().gap(S["space-2"]).flex_wrap().font_family(this.font.mono).text_size(S["type-small"][0]).child(entry.name).child(entry.scorer))
      .child(div().whitespace_normal().text_size(S["type-caption"][0]).text_color(t["text-2"]).child(selectable(`score-${entry.name}-${entry.scorer}`, entry.line)));
    return [v_flex().id("costs-scores").gap(S["space-2"])
      .child(h_flex().gap(S["space-2"]).items_center().flex_wrap()
        .child(this.heading(`Scores by ${costs.by}`, null, t)).child(div().flex_1()).when(chips, (el) => el.children(this.splitChips(t))))
      .child(div().whitespace_normal().text_size(S["type-caption"][0]).text_color(t["text-3"]).child("The pass rate leaves out errors and not applicable."))
      .child(this.width < SCORE_COLUMNS
        ? v_flex().children(table.rows.map(stacked))
        : v_flex().id("costs-scores-table").overflow_x_scrollbar().child(row([table.dimension, "Scorer", "Pass rate", "pass", "fail", "error", "not applicable"], true))
          .children(table.rows.map((entry) => row([entry.name, entry.scorer, entry.rate, ...[entry.pass, entry.fail, entry.error, entry.notApplicable].map(String)], false))))];
  }

  /** One bar per day of both weeks; a dotted line starts this week, so the split never rests on color. @param {any} costs @param {Palette} t */
  days(costs, t) {
    const bars = /** @type {Bar[]} */ (dayBars(costs));
    const top = Math.max(0, ...costs.days.map((/** @type {any} */ day) => day.cost_usd ?? 0));
    const split = bars.findIndex((bar) => bar.current);
    const weekdays = this.width >= WEEKDAYS;
    return v_flex().gap(S["space-1"]).accessibility_label(`Cost per day, last week then this week. ${bars.map((bar) => bar.text).join(". ")}.`)
      .child(div().h(18).text_size(S["type-caption"][0]).text_color(t["text-3"]).child(top ? money(top) : ""))
      .child(h_flex().items_end().border_b(1).border_color(t["control-line"])
        .children(bars.map((bar, index) => v_flex().flex_1().min_w_0().h(CHART).justify_end().items_center()
          .when(index === split && split > 0, (el) => el.border_l(1).border_dashed().border_color(t["text-3"]))
          .accessibility_label(bar.text)
          .child(div().w("64%").rounded_t(2).bg(bar.current ? t["text-2"] : t["tile-muted"]).h(Math.max(bar.fraction * CHART, 1))))))
      .child(h_flex().children(bars.map((bar) => v_flex().flex_1().min_w_0().items_center().text_size(S["type-caption"][0])
        .child(div().text_color(t["text-2"]).child(weekdays ? `${bar.weekday} ${bar.date}` : bar.date))
        .child(div().text_color(t["text-3"]).child(String(bar.merges))))))
      .child(div().whitespace_normal().text_size(S["type-caption"][0]).text_color(t["text-3"])
        .child(`Bars show each day's cost${split > 0 ? ", and this week starts at the dotted line" : ""}. The figure under each day counts its merges.`));
  }

  /** @param {Table} table @param {Palette} t */
  splitRows(table, t) {
    /** @param {string[]} cells @param {boolean} head */
    const row = (cells, head) => h_flex().min_h(head ? 28 : 36).gap(S["space-3"]).items_center().border_b(1).border_color(t.line)
      .text_size(head ? S["type-caption"][0] : S["type-small"][0]).text_color(head ? t["text-3"] : t.text)
      .child(div().w(140).flex_none().truncate().font_family(head ? this.font.ui : this.font.mono).child(cells[0]))
      .child(div().w(80).flex_none().text_right().child(cells[1]))
      .child(div().w(100).flex_none().text_right().child(cells[2]))
      .child(div().flex_1().min_w_0().whitespace_normal().text_color(head ? t["text-3"] : t["text-2"]).child(head ? cells[3] : selectable(`note-${cells[0]}`, cells[3])));
    return v_flex().id("costs-split").overflow_x_scrollbar()
      .child(row([table.dimension, "Cost", "Tokens", "Notes"], true))
      .children(table.rows.map((entry) => row([entry.name, entry.cost, entry.tokens, entry.note], false)));
  }

  /** @param {any} report @param {Palette} t */
  attentionPart(report, t) {
    if (!report.acts && !report.tasks.length && !report.waits.acted && !report.waits.open) {
      return [div().whitespace_normal().text_color(t["text-2"]).child("No attention counted this week. Your answers, dismissals and snoozes on asks count here, and each task's time once it runs.")];
    }
    const view = attentionView(report);
    const { headline } = view;
    const tasks = /** @type {TaskTime[]} */ (view.tasks);
    return [
      v_flex().gap(S["space-2"]).child(this.heading("Your attention", "this week", t))
        .child(h_flex().gap(S["space-3"]).items_center()
          .child(div().flex_none().font_family(this.font.display).font_weight(700).text_size(S["type-pane-count"][0]).child(headline.value))
          .child(v_flex().flex_1().min_w_0().gap(S["space-1"])
            .child(div().whitespace_normal().font_family(this.font.display).font_weight(600).child(headline.label))
            .child(div().whitespace_normal().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(headline.note))))
        .child(this.tiles(view.waits, t)),
      v_flex().gap(S["space-2"]).child(this.heading("Where each task's time went", null, t))
        .child(h_flex().gap(S["space-3"]).flex_wrap().text_size(S["type-caption"][0]).text_color(t["text-2"])
          .children(PARTS.map(([part, label]) => h_flex().gap(S["space-2"]).items_center().child(this.part(part, t).w(18).h(10).rounded(2)).child(label))))
        .children(tasks.map((task) => v_flex().gap(S["space-1"]).py(S["space-1"])
          .child(h_flex().gap(S["space-2"]).items_baseline()
            .child(div().font_family(this.font.mono).text_size(S["type-caption"][0]).child(selectable(`time-${task.key}`, task.key)))
            .child(div().flex_1().min_w_0().truncate().text_size(S["type-small"][0]).text_color(t["text-2"]).child(task.workflow))
            .child(div().text_size(S["type-small"][0]).child(task.wall)))
          .child(this.taskBar(task, t)))),
    ];
  }

  /** @param {string} part @param {Palette} t */
  part(part, t) {
    if (part === "working") return div().bg(t["tile-live"]);
    if (part === "person") return div().relative().child(window.paint_path(WHOLE, Background.pattern_slash(t.wait, 2, 5)).absolute().inset_0());
    if (part === "world") return div().border(1).border_dashed().border_color(t["text-3"]);
    return div().bg(t.raised);
  }

  /** @param {TaskTime} task @param {Palette} t */
  taskBar(task, t) {
    return h_flex().h(12).gap(S["space-1"]).accessibility_label(task.text)
      .children(task.parts.filter((part) => part.fraction > 0).map((part) => this.part(part.part, t).h_full().rounded(2)
        .w(`${part.fraction * 100}%`).accessibility_label(`${part.label}: ${part.value}`)));
  }

  /**
   * @param {string} id @param {string} label @param {boolean} on @param {(cx: import("gpui-kit").Context) => void} action
   * @param {Palette} t @param {boolean} [disabled]
   */
  chip(id, label, on, action, t, disabled = false) {
    return Button.new(id).h(S["pane-chip"]).px(S["space-3"]).flex().flex_none().items_center().rounded(S["radius-full"])
      .selected(on).disabled(disabled).opacity(disabled && !on ? 0.5 : 1)
      .bg(on ? t.emph : t.bg).text_color(on ? t["on-emph"] : t["text-2"]).text_size(S["type-small"][0]).font_weight(500)
      .hover((s) => s.bg(on ? t.emph : t.raised)).focus((s) => s.border(2).border_color(t["action-text"]))
      .on_click((_event, cx) => action(cx)).child(label);
  }
}
