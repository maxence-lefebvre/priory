// The factory's spend against its caps, on one line docked at the bottom of the window; a click
// opens the detail as a popup: each cap with a bar, this week by workflow, and the tasks held at a
// cap. With Over9k's saved Claude limits on the machine that runs priory, Claude's 7-day limit too.
import { View, div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { appHome, closeOverlay, exec, home as herdrHome, openOverlay, props, readTextFile, setShared } from "herdr";
import { priory } from "./priory.js";
import { FONTS, palette, STRUCTURE as S } from "./tokens.js";

const READ_EVERY_MS = 60_000;
// Below this width the line drops cents and Claude's figure, which the detail still shows.
const NARROW = 520;
const WARN = 0.8;
const CLAUDE = ".cache/over9k/usage/claude.json";

/** @typedef {Record<string, import("gpui-kit").Color>} Palette */
/** @typedef {{ scope: string, period: string, cap_usd: number, spent_usd: number | null, task: number | null }} Cap */
/** @typedef {{ task: number, key: string, scope: string, period: string, cap_usd: number, spent_usd: number }} Held */

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
  return { theme: config.theme ?? "indigo", on, home, client: priory({ exec, binary, on, factory: config.factory ?? null }) };
}

const message = (/** @type {unknown} */ error) => (error instanceof Error ? error.message : String(error));
const money = (/** @type {number | null | undefined} */ usd, cents = true) => (usd == null ? "-" : `$${cents ? usd.toFixed(2) : Math.round(usd)}`);
const whole = (/** @type {number} */ usd) => `$${Number.isInteger(usd) ? usd : usd.toFixed(2)}`;
const share = (/** @type {number | null} */ spent, /** @type {number} */ cap) => (spent ?? 0) / cap;

/** Whose cap, as the hold's reason says it: "the factory's", "lane core/intake's". @param {string} scope */
const owner = (scope) => (scope === "factory" ? "the factory's" : `${scope}'s`);

/** Seconds as "3d 4h", "5h 12m" or "12m". @param {number} seconds */
function left(seconds) {
  const s = Math.max(0, Math.round(seconds));
  if (s < 3600) return `${Math.max(1, Math.floor(s / 60))}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
  return `${Math.floor(s / 86400)}d ${Math.floor((s % 86400) / 3600)}h`;
}

/** The local date `costs` keys its days by, now: its window starts at a local midnight. @param {any} costs */
function today(costs) {
  const offset = Date.parse(`${costs.current.since_day}T00:00:00Z`) - Date.parse(costs.current.since);
  return new Date(Date.parse(costs.current.until) + offset).toISOString().slice(0, 10);
}

/** Over9k's saved Claude limits: the 7-day window's used percent, 0 once it reset, else null. @param {string | null} text */
function claudeWeek(text) {
  try {
    const week = JSON.parse(text ?? "null")?.seven_day;
    if (week?.used_percentage == null) return null;
    const resets = week.resets_at ?? null;
    if (resets != null && resets * 1000 <= Date.now()) return { used: 0, resets: null };
    return { used: Math.max(0, Math.min(100, week.used_percentage)), resets };
  } catch {
    return null;
  }
}

export default class PrioryUsage extends View {
  /** @param {any} _props @param {import("gpui-kit").AsyncContext} cx */
  init(_props, cx) {
    this.overlay = props()?.overlay ?? null;
    /** @type {Awaited<ReturnType<typeof connect>> | null} */
    this.priory = null;
    /** @type {any} */
    this.costs = null;
    /** @type {{ used: number, resets: number | null } | null} */
    this.claude = null;
    /** @type {string | null} */
    this.error = null;
    this.width = 1280;
    cx.spawn(async (cx) => {
      for (;;) {
        await this.read();
        cx.notify();
        await cx.sleep(READ_EVERY_MS);
      }
    });
  }

  async read() {
    try {
      this.priory ??= await connect();
      const { client, on, home } = this.priory;
      this.costs = await client.costs();
      this.claude = claudeWeek(await readTextFile(`${home}/${CLAUDE}`, { on }).catch(() => null));
      this.error = null;
    } catch (error) {
      this.error = message(error);
    }
  }

  /** The factory's day or week cap. @param {string} period @returns {Cap | null} */
  factoryCap(period) {
    return this.costs?.caps?.find((/** @type {Cap} */ cap) => cap.scope === "factory" && cap.period === period) ?? null;
  }

  /** @param {Palette} t @param {number} fraction */
  tone(t, fraction) {
    return fraction >= 1 ? t["danger-text"] : fraction >= WARN ? t["warn-text"] : t["text-2"];
  }

  /** @param {import("gpui-kit").Context} cx */
  render(cx) {
    const theme = this.priory?.theme ?? "indigo";
    const t = /** @type {Palette} */ (palette(theme, cx.theme().appearance));
    this.font = FONTS[/** @type {keyof typeof FONTS} */ (theme)] ?? FONTS.indigo;
    return this.overlay ? this.detail(t) : this.line(t);
  }

  /** @param {Palette} t */
  detail(t) {
    const costs = this.costs;
    const body = v_flex().id("usage-detail").size_full().min_h_0().overflow_y_scrollbar().p(14).gap(S["space-3"])
      .bg(t.surface).text_color(t.text).font_family(this.font?.ui ?? FONTS.indigo.ui).text_size(S["type-caption"][0]);
    if (!costs) return body.child(div().text_color(this.error ? t["warn-text"] : t["text-3"]).child(this.error ? `Could not read costs: ${this.error}` : "Reading costs…"));
    const heading = (/** @type {string} */ title, /** @type {string} */ meta = "") => h_flex().justify_between().font_weight(600).text_color(t["text-3"])
      .child(title).when(Boolean(meta), (el) => el.child(div().font_weight(400).child(meta)));
    const section = (/** @type {string} */ id) => v_flex().id(id).gap(6).pt(S["space-3"]).border_t(1).border_color(t.line);
    /** @type {Cap[]} */
    const caps = costs.caps ?? [];
    /** @type {Held[]} */
    const held = costs.held ?? [];
    return body
      .when(caps.length > 0, (el) => el.child(v_flex().id("usage-caps").gap(S["space-2"]).child(heading("Spend caps"))
        .children(caps.map((cap, at) => {
          const fraction = share(cap.spent_usd, cap.cap_usd);
          const spent = cap.period === "task"
            ? (cap.task == null ? "no active task" : `task ${cap.task} ${money(cap.spent_usd)} of ${money(cap.cap_usd)}`)
            : `${money(cap.spent_usd)} of ${money(cap.cap_usd)}`;
          const note = fraction >= 1 && cap.period !== "task" ? `Reached: new runs wait for the next ${cap.period}`
            : cap.period === "task" && cap.task != null ? "The active task under it that spent most" : null;
          return this.bar(`cap-fill-${at}`, `${cap.scope[0].toUpperCase()}${cap.scope.slice(1)}, ${cap.period}`, spent, fraction, t)
            .when(Boolean(note), (el) => el.child(div().text_color(fraction >= 1 ? t["danger-text"] : t["text-3"]).child(note ?? "")));
        }))))
      .child(section("usage-workflows").child(heading("This week by workflow", money(costs.current.cost_usd)))
        .children((costs.split ?? []).map((/** @type {any} */ row) => h_flex().justify_between()
          .child(div().font_family(this.font?.mono ?? FONTS.indigo.mono).child(String(row[costs.by] ?? "-")))
          .child(div().text_color(t["text-2"]).child(money(row.cost_usd))))))
      .when(held.length > 0, (el) => el.child(section("usage-held").child(heading("Held at a cap", String(held.length)))
        .children(held.map((row) => h_flex().id(`held-${row.task}`).gap(S["space-2"]).items_start().px(6).py(4).rounded(S["radius-sm"])
          .hover((s) => s.bg(t.raised)).on_click(() => this.openTask(row.task))
          .child(div().flex_none().font_family(this.font?.mono ?? FONTS.indigo.mono).text_color(t["action-text"]).child(row.key))
          .child(div().flex_1().whitespace_normal().text_color(t["text-2"]).child(`spent ${money(row.spent_usd)} of ${owner(row.scope)} ${row.period} cap of ${money(row.cap_usd)}`))))))
      .when(Boolean(this.claude), (el) => el.child(section("usage-claude").child(heading("Claude subscription"))
        .child(this.bar("claude-fill", "7-day limit", `${Math.round(this.claude?.used ?? 0)}%${this.claude?.resets ? `, resets in ${left(this.claude.resets - Date.now() / 1000)}` : ""}`, (this.claude?.used ?? 0) / 100, t))));
  }

  /** A labelled bar, filled to `fraction`, warn from 80% and danger once reached. @param {string} id @param {string} label @param {string} value @param {number} fraction @param {Palette} t */
  bar(id, label, value, fraction, t) {
    const fill = fraction >= 1 ? t["tile-danger"] : fraction >= WARN ? t["tile-warn"] : t["text-2"];
    return v_flex().gap(4)
      .child(h_flex().justify_between().gap(S["space-2"]).child(div().truncate().child(label)).child(div().flex_none().text_color(t["text-2"]).child(value)))
      .child(div().h(6).w_full().rounded(S["radius-full"]).bg(t.raised).overflow_hidden()
        .child(div().id(id).h_full().rounded(S["radius-full"]).bg(fill).w(`${Math.round(Math.min(1, fraction) * 100)}%`)));
  }

  /** @param {number} task */
  openTask(task) {
    setShared("priory.task.open", { task, at: Date.now() });
    closeOverlay(this.overlay);
  }

  open() {
    openOverlay({ mod: "priory-usage", props: { detail: true }, kind: "popup", width: 380, height: 560 });
  }

  /** @param {Palette} t */
  line(t) {
    const narrow = this.width < NARROW;
    /** @type {{ id: string, text: string, color: import("gpui-kit").Color }[]} */
    const parts = [];
    if (this.costs) {
      const day = this.factoryCap("day"), week = this.factoryCap("week");
      const todaySpent = day ? day.spent_usd : this.costs.days.find((/** @type {any} */ entry) => entry.day === today(this.costs))?.cost_usd ?? 0;
      const weekSpent = week ? week.spent_usd : this.costs.current.cost_usd;
      const spend = (/** @type {number | null} */ spent, /** @type {Cap | null} */ cap, /** @type {string} */ when, /** @type {string} */ short) => (narrow
        ? `${money(spent, false)}${cap ? ` of ${money(cap.cap_usd, false)}` : ""} ${short}`
        : `${money(spent)} ${when}${cap ? ` of ${whole(cap.cap_usd)}` : ""}`);
      parts.push({ id: "usage-today", text: spend(todaySpent, day, "today", "today"), color: day ? this.tone(t, share(todaySpent, day.cap_usd)) : t["text-2"] });
      parts.push({ id: "usage-week", text: spend(weekSpent, week, "this week", "wk"), color: week ? this.tone(t, share(weekSpent, week.cap_usd)) : t["text-2"] });
      const held = this.costs.held?.length ?? 0;
      if (held) parts.push({ id: "usage-held", text: narrow ? `${held} held` : `${held} held at a cap`, color: t.text });
      if (this.claude && !narrow) parts.push({ id: "usage-claude", text: `Claude 7d ${Math.round(this.claude.used)}%`, color: this.tone(t, this.claude.used / 100) });
    }
    const status = this.error ? { text: "Could not read costs", color: t["warn-text"] } : !this.costs ? { text: "Reading costs…", color: t["text-3"] } : null;
    return h_flex().id("usage-line").size_full().px(12).gap(8).items_center().overflow_hidden().whitespace_nowrap()
      .hover((s) => s.bg(t.raised)).on_click(() => this.open())
      .bg(t.bar).border_t(1).border_color(t.line).font_family(this.font?.ui ?? FONTS.indigo.ui).text_size(12)
      .on_size_change(({ width }, cx) => {
        if (Math.abs(width - this.width) < 1) return;
        this.width = width;
        cx.notify();
      })
      .child(div().flex_none().font_weight(600).text_color(t.text).child("Priory"))
      .when(Boolean(status), (el) => el.child(div().flex_none().text_color(status?.color ?? t.text).child(status?.text ?? "")))
      .children(parts.flatMap((part, at) => [
        ...(at ? [div().flex_none().text_color(t["text-3"]).child("·")] : []),
        div().id(part.id).flex_none().text_color(part.color).child(part.text),
      ]));
  }
}
