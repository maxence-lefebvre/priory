// The Add form the Desk and Queue panels open with `N` or their Add button (DESIGN.md, Add).
// One field and the workflow, band and lane chips; Enter runs `priory add`.
import { div, svg } from "gpui-kit";
import { h_flex, v_flex, Button, Input, InputState } from "gpui-base";
import { addStatus } from "./add.js";
import { STRUCTURE as S } from "./tokens.js";

/** @typedef {Record<string, import("gpui-kit").Color>} Palette */
/** @typedef {{ add: (text: string, chips: Record<string, string>) => Promise<string> }} Client */

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
    /** @type {Record<string, string>} the chips the person picked; the rest come from [request] */
    this.picked = {};
    this.input = InputState.new({ placeholder: "What should the factory do?" });
    this.input.on("focus", (_event, cx) => { this.typing = true; cx.notify(); });
    this.input.on("blur", (_event, cx) => { this.typing = false; cx.notify(); });
    this.input.on("change", (_event, cx) => { this.key = null; cx.notify(); });
    this.input.on("submit", (_event, cx) => this.submit(cx));
  }

  /** @param {import("gpui-kit").Context} cx */
  show(cx) {
    this.open = true;
    this.input.focus();
    cx.notify();
  }

  /** @param {import("gpui-kit").Context} cx */
  hide(cx) {
    this.open = false;
    this.typing = false;
    cx.notify();
  }

  status() {
    return addStatus({ text: this.input.value(), sending: this.sending, error: this.error, key: this.key });
  }

  /** The draft and its chips stay until priory takes them. @param {import("gpui-kit").Context} cx */
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

  /**
   * @param {Palette} t @param {string} icons the mod's icon folder
   * @param {{ workflow: string[], band: string[], lane: string[] }} choices
   */
  render(t, icons, choices) {
    const status = this.status();
    /** @param {"workflow" | "band" | "lane"} field @param {string} label */
    const chips = (field, label) => h_flex().gap(S["space-1"]).flex_wrap().items_center()
      .child(div().w(64).flex_none().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(label))
      .children(choices[field].map((value) => {
        const on = this.picked[field] === value;
        return Button.new(`add-${field}-${value}`).h(S["pane-chip"]).px(10).flex().flex_none().items_center().rounded(S["radius-sm"])
          .selected(on).border(1).border_color(on ? t["action-soft"] : t["control-line"]).bg(on ? t["action-soft"] : t.surface)
          .text_size(S["type-caption"][0]).text_color(on ? t["action-text"] : t.text)
          .focus((s) => s.border(2).border_color(t["action-text"]))
          .accessibility_label(`${label}: ${value}${on ? ". Clear" : ""}`)
          .on_click((/** @type {unknown} */ _e, /** @type {import("gpui-kit").Context} */ cx) => {
            if (on) delete this.picked[field];
            else this.picked[field] = value;
            cx.notify();
          })
          .child(value);
      }));
    const tone = { muted: t["text-3"], warn: t["warn-text"], ok: t["ok-text"] }[status.tone];
    return v_flex().id("add-form").flex_none().p(S["space-3"]).gap(S["space-2"]).rounded(S["radius-card"]).bg(t.surface).border(1).border_color(t.line)
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
      .when(choices.workflow.length > 0, (el) => el.child(chips("workflow", "Workflow")))
      .when(choices.band.length > 0, (el) => el.child(chips("band", "Band")))
      .when(choices.lane.length > 0, (el) => el.child(chips("lane", "Lane")))
      .child(div().whitespace_normal().text_size(S["type-caption"][0]).text_color(tone).child(status.text));
  }
}
