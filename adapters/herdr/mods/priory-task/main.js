import { View, div, svg } from "gpui-kit";
import { h_flex, v_flex, Button, TextView } from "gpui-base";
import { appHome, exec, focused, home as herdrHome, layout as shown, panes, readTextFile, run, setShared, shared } from "herdr";
import { showAsks } from "./layout.js";
import { follow, priory, spansHtml } from "./priory.js";
import { FONTS, palette, STRUCTURE as S } from "./tokens.js";
import { taskView } from "./view.js";

const RETRY_MS = 3000;
const ICON = (/** @type {string} */ name) => `mods/priory-task/icons/${name}.svg`;
// A state's tile: its fill, its ink and its icon, as DESIGN.md's state tiles; null draws a dot.
/** @type {Record<string, [string, string, string | null]>} */
const TILES = {
  queued: ["tile-muted", "on-tile", "circle-dashed"], held: ["tile-hold", "on-tile", "pause"], ready: ["tile-live", "on-tile-live", "circle-dot"],
  running: ["tile-live", "on-tile-live", null], asleep: ["tile-muted", "on-tile", "moon"], blocked: ["tile-warn", "on-tile", "hand"],
  done: ["tile-ok", "on-tile", "check"], failed: ["tile-danger", "on-tile", "circle-x"], dropped: ["tile-muted", "on-tile", "minus"],
  moved: ["tile-muted", "on-tile", "arrow-right"], lost: ["tile-warn", "on-tile", "unplug"], ask: ["wait", "on-wait", "ask"],
  answered: ["tile-muted", "on-tile", "check"], withdrawn: ["tile-muted", "on-tile", "minus"],
  // A scorer's verdict: an error means the scorer broke, not the run.
  pass: ["tile-ok", "on-tile", "check"], fail: ["tile-danger", "on-tile", "circle-x"], error: ["tile-warn", "on-tile", "circle-alert"],
  "not applicable": ["tile-muted", "on-tile", "minus"],
};
// A task state's text color (DESIGN.md, State tile); the other states take --text-2.
const ROLE = /** @type {Record<string, string>} */ ({ running: "action-text", ready: "action-text", blocked: "warn-text", failed: "danger-text", done: "ok-text" });
const VERDICT_ROLE = /** @type {Record<string, string>} */ ({ pass: "ok-text", fail: "danger-text", error: "warn-text" });

/** @typedef {Record<string, import("gpui-kit").Color>} Palette */
/** @typedef {ReturnType<typeof taskView>} Task */

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

/** Text with its autolinks; plain text stays a plain child. @param {string} id @param {{ text: string, url?: string }[]} spans */
const linked = (id, spans) => (spans.some((span) => span.url) ? TextView.html(id, spansHtml(spans)) : spans.map((span) => span.text).join(""));

const message = (/** @type {unknown} */ error) => (error instanceof Error ? error.message : String(error));

// One task in detail, opened from another Priory panel; Herdr has no task view of its own.
export default class PrioryTask extends View {
  init() {
    /** @type {{ theme: string, client: ReturnType<typeof priory>, links: { pattern: string, url: string }[] } | null} */
    this.priory = null;
    /** @type {number | null} the task shown */
    this.id = null;
    /** @type {any} its last read, `priory task ID --json` */
    this.task = null;
    /** @type {string | null} */
    this.error = null;
    this.updated = 0;
  }

  // Another Priory mod opens a task here with setShared("priory.task.open", { task, at: Date.now() }).
  /** Reads the task, then again at each revision, until another task opens. @param {number} id @param {import("gpui-kit").Context} cx */
  show(id, cx) {
    [this.id, this.task, this.error] = [id, null, null];
    /** @type {ReturnType<typeof priory>} the client of the last read, which its wait uses */
    let client;
    cx.spawn((cx) => follow({
      read: async () => {
        this.priory ??= await connect();
        client = this.priory.client;
        const task = await client.task(id);
        if (this.id !== id) return null;
        [this.task, this.updated, this.error] = [task, Date.now(), null];
        cx.notify();
        return task.revision;
      },
      wait: (since) => client.wait(since),
      sleep: (ms) => cx.sleep(ms),
      failed: (error) => {
        this.priory = null;
        this.error = message(error);
        cx.notify();
      },
      retry: RETRY_MS,
      stopped: () => this.id !== id,
    }));
  }

  /** @param {import("gpui-kit").Context} cx */
  render(cx) {
    const request = /** @type {{ task?: number } | null} */ (shared("priory.task.open"));
    if (request?.task != null && request.task !== this.id) this.show(request.task, cx);
    const theme = this.priory?.theme ?? "indigo";
    const t = /** @type {Palette} */ (palette(theme, cx.theme().appearance));
    const font = FONTS[/** @type {keyof typeof FONTS} */ (theme)] ?? FONTS.indigo;
    const task = this.task ? taskView(this.task, { now: Date.now(), links: this.priory?.links ?? [] }) : null;
    const note = (/** @type {string} */ text) => div().p(S["space-4"]).whitespace_normal().text_color(t["text-2"]).child(text);
    return v_flex().id("priory-task").size_full().min_h_0().bg(t.bg).text_color(t.text).font_family(font.ui).text_size(S["type-body"][0])
      .when(Boolean(task), (el) => el.child(this.header(/** @type {Task} */ (task), t, font)))
      .when(Boolean(this.error), (el) => el.child(this.offline(t)))
      .child(task ? this.body(task, t, font)
        : this.id == null ? note("Open a task from the Desk to see it here.")
          : this.error ? div() : note(`Loading task ${this.id}…`));
  }

  /** @param {Task} task @param {Palette} t @param {typeof FONTS.indigo} font */
  header(task, t, font) {
    return h_flex().flex_none().h(S["pane-head"]).px(S["space-4"]).gap(S["space-2"]).items_center().border_b(1).border_color(t.line)
      .child(this.tile(task.state, t))
      .child(div().flex_none().font_family(font.mono).text_size(S["type-mono"][0]).text_color(t["text-2"]).child(task.key))
      .child(div().flex_none().text_size(S["type-caption"][0]).text_color(t[ROLE[task.state] ?? "text-2"]).child(task.state))
      .child(div().flex_1())
      // Watch and Attach open the live run's terminal, where Open task opens this panel. Watch cannot type, so it comes first.
      .child(this.terminalButton("watch", "Watch", true, task.id, t))
      .child(this.terminalButton("attach", "Attach", false, task.id, t));
  }

  /** @param {string} id @param {string} label @param {boolean} readOnly @param {number} task @param {Palette} t */
  terminalButton(id, label, readOnly, task, t) {
    return Button.new(id).h(S["control-fine"] - 4).px(S["space-3"]).flex().flex_none().items_center().rounded(S["radius-card"])
      .border(1).border_color(t["control-line"]).bg(t.surface).text_color(t.text).text_size(S["type-small"][0]).font_weight(600)
      .hover((s) => s.bg(t.raised)).focus((s) => s.border(2).border_color(t["action-text"]))
      .on_click(() => setShared("priory.attach", { task, at: Date.now(), readOnly })).child(label);
  }

  // The Asks panel is the one place to answer, as the Desk's Answer… opens it.
  /** @param {number | string} ask @param {Palette} t */
  answerButton(ask, t) {
    return Button.new(`answer-ask:${ask}`).h(S["control-fine"] - 4).px(S["space-3"]).flex().flex_none().items_center().rounded(S["radius-card"])
      .border(1).border_color(t.action).bg(t.action).text_color(t["on-action"]).text_size(S["type-small"][0]).font_weight(600)
      .hover((s) => s.opacity(0.88)).focus((s) => s.border_color(t["on-action"]))
      .on_click(() => {
        setShared("priory.asks.open", { ask, at: Date.now() });
        showAsks(shown(), panes() ?? [], focused()?.space).forEach(([command, args]) => run(command, args));
      }).child("Answer…");
  }

  /** @param {Palette} t */
  offline(t) {
    const minutes = Math.round((Date.now() - this.updated) / 60_000);
    const last = this.task ? ` Last update ${minutes < 1 ? "under a minute" : `${minutes} min`} ago.` : "";
    return div().flex_none().px(S["space-4"]).py(6).whitespace_normal().text_size(S["type-small"][0]).text_color(t["warn-text"]).border_b(1).border_color(t.line)
      .child(`Reconnecting to priory: ${this.error}.${last}`);
  }

  /** @param {Task} task @param {Palette} t @param {typeof FONTS.indigo} font */
  body(task, t, font) {
    /** @param {string} title @param {number} count */
    const heading = (title, count) => h_flex().gap(S["space-2"]).text_size(S["type-caption"][0])
      .child(div().font_weight(600).child(title)).child(div().text_color(t["text-3"]).child(String(count)));
    /** @param {string} id @param {string} tile @param {string} text @param {string | null} note @param {any[]} [below] */
    const line = (id, tile, text, note, below = []) => h_flex().id(id).gap(S["space-2"]).py(S["space-1"]).items_start().border_t(1).border_color(t.line)
      .child(this.tile(tile, t))
      .child(v_flex().flex_1().min_w_0().gap(2)
        .child(div().whitespace_normal().text_size(S["type-small"][0]).child(text))
        .when(Boolean(note), (el) => el.child(div().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(note ?? "")))
        .children(below));
    /** @param {Task["history"][number]["scores"][number]} score */
    const scored = (score) => h_flex().gap(S["space-2"]).pt(S["space-1"]).items_start().border_t(1).border_color(t.line)
      .child(this.tile(score.verdict, t))
      .child(v_flex().flex_1().min_w_0().gap(2)
        .child(h_flex().gap(S["space-1"]).flex_wrap().text_size(S["type-small"][0])
          .child(div().font_family(font.mono).child(score.scorer))
          .child(div().font_weight(600).text_color(t[VERDICT_ROLE[score.verdict] ?? "text-2"]).child(score.verdict)))
        .when(Boolean(score.reason), (el) => el.child(div().whitespace_normal().text_size(S["type-caption"][0]).text_color(t["text-2"]).child(score.reason ?? ""))));
    const pr = task.pr;
    return v_flex().id("task-scroll").flex_1().min_h_0().overflow_y_scrollbar().p(S["space-4"]).gap(S["space-4"])
      .child(v_flex().gap(S["space-2"])
        .child(div().whitespace_normal().font_family(font.display).text_size(S["type-pane-question"][0])
          .line_height(`${S["type-pane-question"][1]}px`).font_weight(650).child(linked("task-title", task.titleSpans)))
        .child(h_flex().gap(S["space-2"]).flex_wrap().text_size(S["type-caption"][0]).text_color(t["text-2"]).children(task.context))
        .when(Boolean(task.reason), (el) => el.child(div().whitespace_normal().text_size(S["type-small"][0])
          .text_color(t[ROLE[task.state] ?? "text-2"]).child(task.reason ?? ""))))
      .when(task.asks.length > 0, (el) => el.child(v_flex().gap(S["space-1"]).child(heading("Asks", task.asks.length))
        .children(task.asks.map((ask) => line(`ask-${ask.id}`, ask.tile, ask.text, ask.note)
          .when(ask.tile === "ask", (el) => el.child(this.answerButton(ask.id, t)))))))
      .when(Boolean(pr) || task.stack.length > 0, (el) => el.child(v_flex().gap(S["space-1"]).child(heading("Pull request", Math.max(1, task.stack.length)))
        .when(Boolean(pr), (el) => el.child(v_flex().gap(2).py(S["space-1"]).border_t(1).border_color(t.line)
          .child(div().whitespace_normal().text_size(S["type-small"][0]).font_weight(600)
            .child(linked("pr-title", [{ text: pr?.title ?? "", ...(pr?.url?.startsWith("https://") && { url: pr.url }) }])))
          .child(div().text_size(S["type-caption"][0]).text_color(t["text-2"]).child(pr?.line ?? ""))
          .when(Boolean(pr?.checks), (el) => el.child(div().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(pr?.checks ?? "")))))
        .children(task.stack.map((pull) => line(`pull-${pull.number}`, pull.state === "merged" ? "done" : pull.state === "closed" ? "dropped" : pull.next ? "ready" : "queued",
          `#${pull.number} ${pull.branch} into ${pull.base}`, pull.next ? `${pull.state}, next to merge` : pull.state)))))
      .when(task.inbox.length > 0, (el) => el.child(v_flex().gap(S["space-1"]).child(heading("Inbox", task.inboxTotal))
        .children(task.inbox.map((event) => line(`event-${event.id}`, event.tile, event.text, event.note)))))
      .when(task.history.length > 0, (el) => el.child(v_flex().gap(S["space-1"]).child(heading("History", task.history.length))
        .children(task.history.map((run) => line(`run-${run.id}`, run.tile, run.text, run.when, run.scores.map(scored))))));
  }

  /** @param {string} state @param {Palette} t */
  tile(state, t) {
    const [fill, ink, icon] = TILES[state] ?? ["tile-muted", "on-tile", null];
    return h_flex().size(18).flex_none().items_center().justify_center().rounded(S["radius-tile"]).bg(t[fill])
      .child(icon ? svg(ICON(icon)).size(12).text_color(t[ink]) : div().size(6).rounded(3).bg(t[ink]));
  }
}
