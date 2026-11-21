import { View } from "gpui-kit";
import { v_flex, TextView } from "gpui-base";
import { appHome, readTextFile } from "herdr";
import { palette, STRUCTURE as S } from "./tokens.js";

// Stands in for a view's panel that has no mod yet, such as tasks, runs or inbox.
const LABELS = { tasks: "Tasks", runs: "Runs", inbox: "Inbox" };

export default class PrioryElsewhere extends View {
  /** @param {{ panel?: string }} props @param {import("gpui-kit").Context} cx */
  init(props, cx) {
    this.panel = props?.panel ?? "panel";
    this.theme = "indigo";
    /** @type {string | null} the web view's address, from settings.json's "priory" object */
    this.web = null;
    cx.spawn(async (cx) => {
      try {
        const config = JSON.parse(await readTextFile(`${appHome()}/settings.json`) ?? "{}").priory ?? {};
        this.theme = config.theme ?? "indigo";
        this.web = typeof config.web === "string" ? config.web.replace(/\/+$/, "") : null;
      } catch {
        this.web = null;
      }
      cx.notify();
    });
  }

  /** @param {import("gpui-kit").Context} cx */
  render(cx) {
    const t = /** @type {Record<string, import("gpui-kit").Color>} */ (palette(this.theme, cx.theme().appearance));
    const name = /** @type {Record<string, string>} */ (LABELS)[this.panel] ?? this.panel;
    const where = this.web
      ? `<a href="${this.web}/${this.panel}">Open ${name} in the web view</a>`
      : `Set "web" in settings.json's "priory" object to link it in the web view.`;
    return v_flex().size_full().p(S["space-4"]).gap(S["space-2"]).bg(t.bg).text_size(S["type-small"][0]).text_color(t["text-2"])
      .child(`${name} has no Herdr panel yet.`)
      .child(TextView.html("elsewhere-link", where));
  }
}
