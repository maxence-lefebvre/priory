// The Add form the Desk and Queue panels open with `N` or their Add button (DESIGN.md, Add).
// One field and the workflow, band and lane dropdowns; Enter runs `priory add`.
import { div, svg } from "gpui-kit";
import { h_flex, v_flex, Button, Input, InputState, Popover } from "gpui-base";
import { addStatus } from "./add.js";
import { STRUCTURE as S } from "./tokens.js";

/** @typedef {Record<string, import("gpui-kit").Color>} Palette */
/** @typedef {{ add: (text: string, picked: Record<string, string>) => Promise<string> }} Client */

export class AddForm {
  /**
   * @param {() => Promise<Client>} client
   * @param {(key: string, cx: import("gpui-kit").Context) => void} added runs once priory took the task
   */
  constructor(client, added) {
    this.client = client;
    this.added = added;
    this.open = false;
    this.typing = false;
    this.sending = false;
    /** @type {string | null} */
    this.error = null;
    /** @type {string | null} */
    this.key = null;
    /** @type {Record<string, string>} what the person picked; the rest comes from [request] */
    this.picked = {};
    /** @type {string | null} the field whose menu is open */
    this.menu = null;
    /** @type {import("gpui-kit").FocusHandle | null} */
    this.focus = null;
    this.input = InputState.new({ placeholder: "What should the factory do?" });
    this.input.on("focus", (_event, cx) => { this.typing = true; cx.notify(); });
    this.input.on("blur", (_event, cx) => { this.typing = false; cx.notify(); });
    this.input.on("change", (_event, cx) => { this.key = null; cx.notify(); });
    this.input.on("submit", (_event, cx) => this.submit(cx));
  }

  /** @param {import("gpui-kit").Context} cx */
  show(cx) {
    this.open = true;
    this.focus ??= cx.focus_handle();
    this.input.focus();
    cx.notify();
  }

  /** @param {import("gpui-kit").Context} cx */
  hide(cx) {
    this.open = false;
    this.typing = false;
    this.menu = null;
    cx.notify();
  }

  status() {
    return addStatus({ text: this.input.value(), sending: this.sending, error: this.error, key: this.key });
  }

  /** The draft and its picks stay until priory takes them. @param {import("gpui-kit").Context} cx */
  submit(cx) {
    if (!this.status().ready) return;
    const text = this.input.value().trim();
    const picked = { ...this.picked };
    this.sending = true;
    this.error = null;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const key = await (await this.client()).add(text, picked);
        // What the person typed or picked while it sent stays for the next add.
        if (this.input.value().trim() === text) this.input.set_value("");
        if (JSON.stringify(this.picked) === JSON.stringify(picked)) this.picked = {};
        this.key = key;
        this.added(key, cx);
      } catch (error) {
        this.error = error instanceof Error ? error.message : String(error);
      }
      this.sending = false;
      cx.notify();
    });
  }

  /** Keeps a key pressed on a control from the panel's shortcuts, and Esc from the form while a menu is open. @param {import("gpui-kit").KeyEvent} event @param {import("gpui-kit").Context} cx */
  keyDown(event, cx) {
    // Stopping a key the field has yet to type would lose it.
    if (this.typing) return;
    if (event.key === "escape") {
      if (!this.menu) return;
      this.menu = null;
      cx.notify();
    }
    cx.stop_propagation();
  }

  /**
   * @param {Palette} t @param {string} icons the mod's icon folder
   * @param {{ workflow: string[], band: string[], lane: string[], unpicked: Record<"workflow" | "band" | "lane", string> }} choices
   */
  render(t, icons, choices) {
    const status = this.status();
    /** @param {"workflow" | "band" | "lane"} field @param {string} label */
    const dropdown = (field, label) => {
      const value = this.picked[field];
      const open = this.menu === field;
      const close = (/** @type {import("gpui-kit").Context} */ cx) => { this.menu = null; cx.notify(); };
      const row = (/** @type {string} */ id, /** @type {string} */ name, /** @type {boolean} */ on) => Button.new(id).h(S["control-fine"]).px(S["space-2"]).flex().flex_none().items_center()
        .rounded(S["radius-sm"]).selected(on).text_size(S["type-small"][0]).text_color(on ? t["action-text"] : t.text)
        .hover((s) => s.bg(t.raised)).focus((s) => s.bg(t.raised)).child(name);
      const shown = value ?? choices.unpicked[field];
      const trigger = Button.new(`add-${field}`).h(S["pane-chip"]).px(S["space-3"]).flex().flex_none().items_center().gap(S["space-1"]).rounded(S["radius-sm"])
        .selected(open).border(1).border_color(value ? t["action-soft"] : t["control-line"]).bg(value ? t["action-soft"] : t.surface)
        .text_size(S["type-small"][0]).focus((s) => s.border(2).border_color(t["action-text"]))
        .accessibility_label(`${label}: ${shown}`)
        .child(div().text_color(t["text-3"]).child(label))
        .child(div().font_weight(500).text_color(value ? t["action-text"] : t.text).child(shown))
        .child(svg(`${icons}/chevron-down.svg`).size(12).text_color(value ? t["action-text"] : t["text-3"]));
      return Popover.new(`add-${field}-menu`).trigger(trigger).anchor("top_left").open(open)
        .on_open_change((/** @type {boolean} */ next, /** @type {import("gpui-kit").Context} */ cx) => {
          if (next) this.menu = field;
          // The menu opened next may have replaced this one before its close arrives.
          else if (this.menu === field) this.menu = null;
          cx.notify();
        })
        .content(v_flex().id(`add-${field}-list`).min_w(200).max_h(320).overflow_y_scrollbar().p(S["space-1"]).gap(S["space-1"]).rounded(S["radius-md"]).bg(t.surface).border(1).border_color(t["control-line"])
          .child(row(`add-clear-${field}`, choices.unpicked[field], !value)
            .on_click((/** @type {unknown} */ _e, /** @type {import("gpui-kit").Context} */ cx) => { delete this.picked[field]; close(cx); }))
          .children(choices[field].map((name) => row(`add-${field}-${name}`, name, name === value)
            .on_click((/** @type {unknown} */ _e, /** @type {import("gpui-kit").Context} */ cx) => { this.picked[field] = name; close(cx); }))));
    };
    const tone = { muted: t["text-3"], warn: t["warn-text"], ok: t["ok-text"] }[status.tone];
    return v_flex().id("add-form").flex_none().p(S["space-3"]).gap(S["space-2"]).rounded(S["radius-card"]).bg(t.surface).border(1).border_color(t.line)
      .when(Boolean(this.focus), (el) => el.track_focus(/** @type {import("gpui-kit").FocusHandle} */ (this.focus)))
      .on_key_down((/** @type {import("gpui-kit").KeyEvent} */ event, /** @type {import("gpui-kit").Context} */ cx) => this.keyDown(event, cx))
      .child(h_flex().gap(S["space-2"]).items_center()
        .child(div().flex_1().h(S["control-fine"]).px(S["space-2"]).rounded(S["radius-md"]).border(1).border_color(t["control-line"]).bg(t.bg)
          .child(Input.new(this.input).h_full().w_full().text_size(S["type-small"][0]).text_color(t.text)))
        .child(Button.new("add-send").disabled(!status.ready).h(S["control-fine"]).px(S["space-3"]).flex().flex_none().items_center().rounded(S["radius-md"])
          .bg(t.action).text_color(t["on-action"]).text_size(S["type-caption"][0]).font_weight(600)
          .when(!status.ready, (el) => el.opacity(0.5)).focus((s) => s.border(2).border_color(t["on-action"]))
          .on_click((/** @type {unknown} */ _e, /** @type {import("gpui-kit").Context} */ cx) => this.submit(cx)).child(this.sending ? "Adding…" : "Add"))
        .child(Button.new("add-close").size(S["control-fine"]).flex().flex_none().items_center().justify_center().rounded(S["radius-sm"])
          .accessibility_label("Close Add").hover((s) => s.bg(t.raised)).focus((s) => s.bg(t.raised))
          .on_click((/** @type {unknown} */ _e, /** @type {import("gpui-kit").Context} */ cx) => this.hide(cx))
          .child(svg(`${icons}/x.svg`).size(14).text_color(t["text-2"]))))
      .child(h_flex().gap(S["space-2"]).flex_wrap()
        .when(choices.workflow.length > 0, (el) => el.child(dropdown("workflow", "Workflow")))
        .when(choices.band.length > 0, (el) => el.child(dropdown("band", "Band")))
        .when(choices.lane.length > 0, (el) => el.child(dropdown("lane", "Lane"))))
      .child(div().whitespace_normal().text_size(S["type-caption"][0]).text_color(tone).child(status.text));
  }
}
