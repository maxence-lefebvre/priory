import { View, div, svg } from "gpui-kit";
import { h_flex, v_flex, Button } from "gpui-base";
import { appHome, exec, home as herdrHome, readTextFile, setShared } from "herdr";
import { follow, priory } from "./priory.js";
import { FONTS, palette, STRUCTURE as S } from "./tokens.js";
import { eventWords, neverOwned, routeAction, unheardActs } from "./view.js";

const RETRY_MS = 3000;
// The events a group shows, newest first, as the web view's Inbox does.
const SHOWN = 5;
const ICON = (/** @type {string} */ name) => `mods/priory-inbox/icons/${name}.svg`;
// A task state's tile: its fill, its ink and its icon, as the Task panel's; null draws a dot.
/** @type {Record<string, [string, string, string | null]>} */
const TILES = {
  queued: ["tile-muted", "on-tile", "circle-dashed"], held: ["tile-hold", "on-tile", "pause"], ready: ["tile-live", "on-tile-live", "circle-dot"],
  running: ["tile-live", "on-tile-live", null], asleep: ["tile-muted", "on-tile", "moon"], blocked: ["tile-warn", "on-tile", "hand"],
  done: ["tile-ok", "on-tile", "check"], failed: ["tile-danger", "on-tile", "circle-x"], dropped: ["tile-muted", "on-tile", "minus"],
};

/** @typedef {Record<string, import("gpui-kit").Color>} Palette */
/** @typedef {{ resource: string, task: { id: number, key: string, state: string, workflow: string }, events: any[], request_text: string | null }} Group */
/** @typedef {{ task: Group["task"], events: any[], follow_up: string | null, request_text: string | null }} Unheard */

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
  return { theme: config.theme ?? "indigo", client: priory({ exec, binary, on, factory: config.factory ?? null }) };
}

const message = (/** @type {unknown} */ error) => (error instanceof Error ? error.message : String(error));
const when = (/** @type {string} */ iso) => new Date(iso).toLocaleString("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23" });

export default class PrioryInbox extends View {
  /** @param {unknown} _props @param {import("gpui-kit").AsyncContext} cx */
  init(_props, cx) {
    /** @type {Awaited<ReturnType<typeof connect>> | null} */
    this.priory = null;
    /** @type {{ owned: Group[], other: { source: string, events: number }[], unheard?: Unheard[] } | null} */
    this.groups = null;
    /** @type {string | null} */
    this.error = null;
    /** @type {string | null} the group whose action is in flight, or whose result no read shows yet */
    this.busy = null;
    /** @type {number | null} the revision of the groups shown */
    this.revision = null;
    /** Counts reads; an action's result shows from the read numbered `settles` on. */
    this.reads = 0;
    this.settles = 0;
    /** @type {Map<string, string>} what the last action on each group did */
    this.notices = new Map();
    this.font = FONTS.indigo;
    /** @type {ReturnType<typeof priory>} the client of the last read, which its wait uses */
    let client;
    cx.spawn((cx) => follow({
      read: async () => {
        this.priory ??= await connect();
        client = this.priory.client;
        const at = ++this.reads;
        const read = await client.unrouted();
        [this.groups, this.revision, this.error] = [read.unrouted, read.revision, null];
        if (at >= this.settles) this.busy = null;
        cx.notify();
        return read.revision;
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
   * Runs one action on the group `key` names; its notice says what happened. The buttons stay
   * disabled until a read started after the action shows its result; a refused action frees them at once.
   * @param {string} key @param {(client: ReturnType<typeof priory>) => Promise<string>} verb @param {import("gpui-kit").Context} cx
   */
  act(key, verb, cx) {
    if (this.busy || !this.priory) return;
    const client = this.priory.client;
    this.busy = key;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        this.notices.set(key, await verb(client));
        this.settles = this.reads + 1;
      } catch (error) {
        this.notices.set(key, `Could not route it: ${message(error)}.`);
        this.busy = null;
      }
      cx.notify();
    });
  }

  /** @param {import("gpui-kit").Context} cx */
  render(cx) {
    const theme = this.priory?.theme ?? "indigo";
    const t = /** @type {Palette} */ (palette(theme, cx.theme().appearance));
    this.font = FONTS[/** @type {keyof typeof FONTS} */ (theme)] ?? FONTS.indigo;
    const count = this.groups ? this.groups.owned.reduce((sum, group) => sum + group.events.length, 0) : null;
    return v_flex().id("priory-inbox").size_full().min_h_0().bg(t.bg).text_color(t.text).font_family(this.font.ui).text_size(S["type-body"][0])
      .child(h_flex().flex_none().h(S["pane-head"]).px(S["space-4"]).gap(S["space-2"]).items_center().border_b(1).border_color(t.line)
        .child(div().font_weight(650).child("Inbox"))
        .child(div().flex_1())
        .child(h_flex().h(S["pane-chip"]).px(10).gap(6).items_center().rounded(S["radius-full"]).bg(t.emph).text_color(t["on-emph"])
          .text_size(S["type-small"][0]).font_weight(500).child("Unrouted").when(count !== null, (el) => el.child(String(count)))))
      .when(Boolean(this.error), (el) => el.child(div().flex_none().px(S["space-4"]).py(6).whitespace_normal().text_size(S["type-small"][0])
        .text_color(t["warn-text"]).border_b(1).border_color(t.line)
        .child(`Could not read the unrouted events: ${this.error}. Check that priory runs, then it reads again on its own.`)))
      .child(v_flex().id("inbox-body").flex_1().min_h_0().overflow_y_scrollbar().p(S["space-4"]).gap(S["space-4"]).children(this.body(t)));
  }

  /** @param {Palette} t */
  body(t) {
    if (!this.groups) return this.error ? [] : [div().text_color(t["text-2"]).child("Reading the unrouted events…")];
    const rest = neverOwned(this.groups.other);
    const restLine = rest ? [div().whitespace_normal().text_size(S["type-small"][0]).text_color(t["text-3"]).child(rest)] : [];
    const unheard = this.groups.unheard ?? [];
    const heard = unheard.length ? [this.heading("Unheard", "no agent read them", t), ...unheard.map((group) => this.unheard(group, t)), this.heading("Unrouted", "reached no task", t)] : [];
    if (!this.groups.owned.length) {
      return [...heard, div().whitespace_normal().text_color(t["text-2"]).child("No event of a task's resource went unrouted. An event about an ended task's pull request or issue shows here."), ...restLine];
    }
    return [...heard, ...this.groups.owned.map((group) => this.card(group.resource, group, this.route(group, t), t)), ...restLine];
  }

  /** A section's title and its note. @param {string} title @param {string} note @param {Palette} t */
  heading(title, note, t) {
    return h_flex().justify_between().items_baseline().text_size(S["type-caption"][0])
      .child(div().font_weight(600).text_color(t["text-2"]).child(title)).child(div().text_color(t["text-3"]).child(note));
  }

  /**
   * A task's unheard events, with what gets them to an agent (view.js, unheardActs): the say
   * command for an asleep task, since Say stays on the web; Resume for a blocked one; an ended
   * one's route. @param {Unheard} group @param {Palette} t
   */
  unheard(group, t) {
    const { task } = group;
    const key = `unheard-${task.id}`;
    const acts = unheardActs(task);
    const line = (/** @type {string} */ words) => div().whitespace_normal().text_size(S["type-small"][0]).text_color(t["text-2"]).child(words);
    const action = acts.includes("resume")
      ? h_flex().gap(S["space-2"]).items_center()
        .child(this.button(key, `Resume task ${task.id}`, async (client) => { await client.resume(task.id); return `Resumed task ${task.id}.`; }, t))
        .child(line("It reads them when it resumes."))
      : acts.includes("say") ? line(group.follow_up ?? "").font_family(this.font.mono)
        : acts.length ? this.route({ ...group, resource: task.key }, t, acts[0], key)
          : line("Its next run reads them.");
    return this.card(task.key, { ...group, resource: task.key }, action, t, key);
  }

  /**
   * A group's card: its `title`, its task, its newest events, then `action`; `key` names its notice.
   * @param {string} title @param {Group} group @param {any} action @param {Palette} t
   */
  card(title, group, action, t, key = group.resource) {
    const { task } = group;
    const notice = this.notices.get(key);
    return v_flex().gap(S["space-2"]).p(S["space-3"]).rounded(S["radius-md"]).border(1).border_color(t.line).bg(t.surface)
      .child(h_flex().gap(S["space-2"]).items_baseline()
        .child(div().flex_1().min_w_0().truncate().font_family(this.font.mono).text_size(S["type-small"][0]).child(title))
        .child(div().text_size(S["type-small"][0]).text_color(t["text-2"]).child(String(group.events.length))))
      .child(Button.new(`open-${key}`).h(S["pane-tile"] + 8).flex().gap(S["space-2"]).items_center().rounded(S["radius-sm"])
        .text_size(S["type-small"][0]).text_color(t["text-2"]).hover((s) => s.bg(t.raised)).focus((s) => s.border(2).border_color(t["action-text"]))
        .accessibility_label(`Open task ${task.id}`)
        .on_click(() => setShared("priory.task.open", { task: task.id, at: Date.now() }))
        .child(this.tile(task.state, t)).child(`task ${task.id}`).child(task.state)
        .child(div().min_w_0().truncate().font_family(this.font.mono).child(task.key)))
      .children(group.events.slice(0, SHOWN).map((event) => v_flex().gap(2).text_size(S["type-small"][0])
        .child(h_flex().gap(6).items_baseline()
          .child(div().font_weight(600).child(eventWords(event.type)))
          .child(div().flex_1().min_w_0().truncate().child(event.subject ?? ""))
          .child(div().flex_none().text_size(S["type-caption"][0]).text_color(t["text-3"]).child(when(event.time))))
        .when(Boolean(event.line), (el) => el.child(div().whitespace_normal().text_color(t["text-2"]).child(event.line)))))
      .when(group.events.length > SHOWN, (el) => el.child(div().text_size(S["type-small"][0]).text_color(t["text-3"]).child(`and ${group.events.length - SHOWN} older`)))
      .child(action)
      .when(Boolean(notice), (el) => el.child(div().whitespace_normal().text_size(S["type-small"][0]).text_color(t["text-2"]).accessibility_label(notice ?? "").child(notice ?? "")));
  }

  /**
   * The way to route the group's next event, by its owner's state (view.js, routeAction).
   * @param {Group} group @param {Palette} t @param {string} action @param {string} key
   */
  route(group, t, action = routeAction(group.task), key = group.resource) {
    const { task } = group;
    if (action === "request" && !group.request_text) {
      return div().whitespace_normal().text_size(S["type-small"][0]).text_color(t["text-3"]).child("No new task can take this resource from here: start one from its thread.");
    }
    /** @type {Record<string, [string, (client: ReturnType<typeof priory>) => Promise<string>]>} */
    const verbs = {
      revive: [`Revive task ${task.id}`, async (client) => { await client.revive(task.id); return `Revived task ${task.id}. Its next event reaches it.`; }],
      request: ["Request a task", async (client) => `Requested ${await client.add(group.request_text ?? "", { workflow: task.workflow })}. It owns this resource's next event.`],
      subscribe: [`Subscribe task ${task.id}`, async (client) => `Subscribed: ${(await client.subscribe(task.id, group.resource, this.revision)).trim()}. Its next event goes to task ${task.id}.`],
    };
    const [label, verb] = verbs[action];
    return h_flex().child(this.button(`route-${key}`, label, verb, t, key));
  }

  /**
   * An action's button, disabled while any action waits for its result.
   * @param {string} id @param {string} label @param {(client: ReturnType<typeof priory>) => Promise<string>} verb @param {Palette} t @param {string} key
   */
  button(id, label, verb, t, key = id) {
    const off = this.busy !== null || !this.priory;
    return Button.new(id).h(S["control-fine"]).px(S["space-3"]).flex().items_center().rounded(S["radius-sm"])
      .border(1).border_color(t["control-line"]).bg(t.bg).text_size(S["type-small"][0]).font_weight(500)
      .disabled(off).opacity(off ? 0.5 : 1).hover((s) => s.bg(t.raised)).focus((s) => s.border(2).border_color(t["action-text"]))
      .on_click((_event, cx) => this.act(key, verb, cx)).child(label);
  }

  /** @param {string} state @param {Palette} t */
  tile(state, t) {
    const [fill, ink, icon] = TILES[state] ?? ["tile-muted", "on-tile", "circle-help"];
    return h_flex().size(S["pane-tile"]).flex_none().items_center().justify_center().rounded(S["radius-tile"]).bg(t[fill]).accessibility_label(state)
      .child(icon ? svg(ICON(icon)).size(14).text_color(t[ink]) : div().size(8).rounded(999).bg(t[ink]));
  }
}
