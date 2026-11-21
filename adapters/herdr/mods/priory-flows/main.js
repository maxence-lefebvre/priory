import { Background, PathBuilder, View, div, svg } from "gpui-kit";
import { h_flex, v_flex, Button } from "gpui-base";
import { appHome, exec, home as herdrHome, readTextFile } from "herdr";
import { pathCalls } from "./path.js";
import { priory } from "./priory.js";
import { FONTS, palette, STRUCTURE as S } from "./tokens.js";
import { edgeStyle, KINDS, nodeShape, timeline, ungatedPairs } from "./view.js";

// A drawing is at most this wide (DESIGN.md, Flows panel).
const MAX_WIDTH = 560;
// gpui-kit turns no text, so a lane's outcome sits beside the lane, in this much room.
const LABEL_ROOM = 76;
const ICON = (/** @type {string} */ name) => `mods/priory-flows/icons/${name}.svg`;
const KIND_WORD = { read: "read", reversible: "reversible write", irreversible: "irreversible write", ask: "ask", answer: "answer", verdict: "verdict" };
const BY_WORD = { code: "code decides", model: "a model decides", human: "a person decides", asleep: "asleep, waiting" };
const BY = /** @type {(keyof typeof BY_WORD)[]} */ (Object.keys(BY_WORD));

/** @typedef {Record<string, import("gpui-kit").Color>} Palette */
/** @typedef {typeof FONTS.indigo} Fonts */
/** @typedef {ReturnType<typeof timeline>} Drawing */
/** @typedef {Drawing["nodes"][number]} Node */
/** @typedef {{ from: string, to: string, outcome: string, by: string, d: string, mid: { x: number, y: number }, back: boolean, vertical: boolean, ungated: boolean }} Edge */

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
const clip = (/** @type {string} */ text, /** @type {number} */ size) => (text.length > size ? `${text.slice(0, size - 1)}...` : text);
const word = (/** @type {Record<string, string>} */ words, /** @type {string} */ key) => words[key] ?? key;

/** gpui-kit geometry from an SVG path of view.js, scaled. @param {PathBuilder} builder @param {string} d @param {number} scale */
function build(builder, d, scale) {
  // @ts-ignore each call names a PathBuilder method with its arguments
  return pathCalls(d, scale).reduce((path, [method, ...args]) => path[method](...args), builder).build();
}

/** A filled arrowhead at an edge's end, pointing along its last curve. @param {string} d @param {number} scale */
function arrowhead(d, scale) {
  const last = pathCalls(d, scale).at(-1) ?? ["move_to", 0, 0];
  const [x, y] = /** @type {number[]} */ (last.slice(1, 3));
  const [fromX, fromY] = last[0] === "cubic_bezier_to" ? /** @type {number[]} */ (last.slice(5, 7)) : [x, y - 1];
  const length = Math.hypot(x - fromX, y - fromY) || 1;
  const [ux, uy] = [(x - fromX) / length, (y - fromY) / length];
  const size = 8;
  return PathBuilder.fill().add_polygon([[x, y], [x - ux * size - uy * size / 2, y - uy * size + ux * size / 2], [x - ux * size + uy * size / 2, y - uy * size - ux * size / 2]]).build();
}

export default class PrioryFlows extends View {
  /** @param {unknown} _props @param {import("gpui-kit").AsyncContext} cx */
  init(_props, cx) {
    /** @type {Awaited<ReturnType<typeof connect>> | null} */
    this.priory = null;
    /** @type {any} */
    this.flows = null;
    /** @type {string | null} */
    this.error = null;
    this.loading = false;
    this.width = 420;
    /** @type {Set<string>} the workflows whose steps show as text */
    this.asText = new Set();
    /** @type {Fonts} */
    this.font = FONTS.indigo;
    this.load(cx);
  }

  /** Reads the factory's flows once; the factory changes only when its files do. */
  /** @param {import("gpui-kit").AsyncContext | import("gpui-kit").Context} cx */
  load(cx) {
    if (this.loading) return;
    this.loading = true;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        this.priory ??= await connect();
        this.flows = await this.priory.client.flows();
        this.error = null;
      } catch (error) {
        this.priory = null;
        this.error = message(error);
      }
      this.loading = false;
      cx.notify();
    });
  }

  /** @param {import("gpui-kit").Context} cx */
  render(cx) {
    const theme = this.priory?.theme ?? "indigo";
    const t = /** @type {Palette} */ (palette(theme, cx.theme().appearance));
    this.font = FONTS[/** @type {keyof typeof FONTS} */ (theme)] ?? FONTS.indigo;
    const pad = this.width >= S["pane-split"] ? S["space-5"] : S["space-4"];
    return v_flex().id("priory-flows").size_full().min_h_0().bg(t.bg).text_color(t.text).font_family(this.font.ui).text_size(S["type-body"][0])
      .on_size_change(({ width }, cx) => {
        if (Math.abs(width - this.width) < 1) return;
        this.width = width;
        cx.notify();
      })
      .child(h_flex().flex_none().h(S["pane-head"]).px(S["space-4"]).gap(S["space-2"]).items_center().border_b(1).border_color(t.line)
        .child(div().font_weight(650).child("Flows")).child(div().flex_1())
        .when(Boolean(this.flows) && !this.loading, (el) => el.child(this.secondary("reload", "Read again", (cx) => this.load(cx), t))))
      .child(v_flex().id("flows-body").flex_1().min_w_0().min_h_0().overflow_y_scrollbar().px(pad).pt(pad).pb(32).gap(S["space-5"])
        .children(this.body(t, pad)));
  }

  /** @param {Palette} t @param {number} pad */
  body(t, pad) {
    const note = (/** @type {string} */ text) => div().whitespace_normal().text_color(t["text-2"]).child(text);
    if (!this.flows && this.error) {
      return [note(`The flows did not load: ${this.error}. Run priory check --explain meanwhile.`).text_color(t["warn-text"]),
        h_flex().child(this.secondary("retry", "Retry", (cx) => this.load(cx), t))];
    }
    if (!this.flows) return [note("Loading the flows.")];
    const { workflows, paths } = this.flows;
    if (!workflows.length) return [note("This factory has no workflows. Add one under workflows/.")];
    const room = this.width - 2 * pad;
    return [
      ...(this.error ? [note(`Could not read the flows again: ${this.error}. This is the last read.`).text_color(t["warn-text"])] : []),
      this.legend(t),
      ...workflows.map((/** @type {any} */ workflow) => this.workflow(workflow, paths, room, t)),
    ];
  }

  /** The six node shapes, the four line styles and the highlight, so no reader has to guess a mark. @param {Palette} t */
  legend(t) {
    const item = (/** @type {any} */ mark, /** @type {string} */ label) => h_flex().gap(6).items_center().child(mark).child(label);
    return v_flex().gap(S["space-2"])
      .child(div().text_size(S["type-small"][0]).font_weight(650).child("Key"))
      .child(h_flex().gap_x(S["space-3"]).gap_y(S["space-1"]).flex_wrap().text_size(S["type-caption"][0]).text_color(t["text-2"])
        .children(KINDS.map((kind) => item(div().w(45).h(16).relative()
          .child(window.paint_path(build(PathBuilder.fill(), nodeShape(kind, 45, 16).d, 1), this.fill(kind, t)).absolute().inset_0())
          .child(window.paint_path(build(PathBuilder.stroke(1), nodeShape(kind, 45, 16).d, 1), kind === "irreversible" ? t["danger-text"] : t["control-line"]).absolute().inset_0()),
        word(KIND_WORD, kind))))
        .children(BY.map((by) => item(div().w(48).h(8).relative()
          .child(window.paint_path(this.dashed(PathBuilder.stroke(edgeStyle(by).width), edgeStyle(by).dash).move_to(0, 4).line_to(48, 4).build(), this.ink(by, t)).absolute().inset_0()),
        BY_WORD[by])))
        .child(item(div().w(48).h(12).relative()
          .child(window.paint_path(PathBuilder.stroke(8).move_to(0, 6).line_to(48, 6).build(), Background.solid(t["danger-text"]).opacity(0.22)).absolute().inset_0()),
        "no person's decision before an irreversible step")));
  }

  /** @param {any} workflow @param {{ workflow: string, path: string[] }[]} paths @param {number} room @param {Palette} t */
  workflow(workflow, paths, room, t) {
    const drawing = timeline(workflow, ungatedPairs(workflow.name, paths));
    const edges = /** @type {Edge[]} */ (drawing.edges);
    const lanes = edges.some((edge) => edge.vertical) ? LABEL_ROOM : 0;
    const scale = Math.min(1, MAX_WIDTH / drawing.width, Math.max(0.5, (room - lanes) / drawing.width));
    const open = paths.filter((entry) => entry.workflow === workflow.name);
    const text = this.asText.has(workflow.name);
    return v_flex().gap(S["space-2"])
      .child(h_flex().gap(6).items_baseline().text_size(S["type-small"][0]).font_weight(650)
        .child(div().font_family(this.font.mono).child(workflow.name))
        .child(div().font_weight(400).text_color(t["text-3"]).child(`${workflow.steps.length} steps`)))
      .child(div().id(`diagram-${workflow.name}`).relative().flex_none().w(drawing.width * scale + lanes).h(drawing.height * scale)
        .accessibility_label(drawing.text)
        .children(edges.map((edge) => this.edge(edge, scale, t)))
        .children(drawing.nodes.map((node) => this.node(node, scale, t))))
      .when(open.length > 0, (el) => el.child(v_flex().gap(S["space-1"]).text_size(S["type-small"][0]).text_color(t["danger-text"])
        .child(h_flex().gap(6).items_center().child(svg(ICON("lock")).size(14).flex_none()).child("Reaches an irreversible step with no person's decision:"))
        .children(open.map((entry) => div().pl(20).whitespace_normal().font_family(this.font.mono).text_size(S["type-caption"][0]).child(entry.path.join(" > "))))))
      .child(Button.new(`text-${workflow.name}`).h(S["control-fine"]).flex().items_center().gap(6).rounded(S["radius-sm"]).px(S["space-1"])
        .text_size(S["type-small"][0]).text_color(t["action-text"]).hover((s) => s.bg(t.raised)).focus((s) => s.border(2).border_color(t["action-text"]))
        .accessibility_label(text ? "Hide the steps as text" : "Show the steps as text")
        .on_click((_event, cx) => {
          if (text) this.asText.delete(workflow.name);
          else this.asText.add(workflow.name);
          cx.notify();
        })
        .child(text ? "Hide steps as text" : "Steps as text"))
      .when(text, (el) => el.child(v_flex().gap(S["space-1"]).text_size(S["type-small"][0]).text_color(t["text-2"])
        .children(workflow.steps.map((/** @type {any} */ step, /** @type {number} */ index) => div().whitespace_normal()
          .child(`${index + 1}. ${step.id}, ${word(KIND_WORD, step.kind)}, decided by ${step.decides}. ${step.edges
            .map((/** @type {any} */ next) => `${next.outcome} leads to ${next.to} (${word(BY_WORD, next.by)}).`).join(" ")}`)))));
  }

  /** @param {string} kind @param {Palette} t */
  fill(kind, t) {
    if (kind === "ask" || kind === "answer") return t["action-soft"];
    return kind === "verdict" ? t.raised : t.surface;
  }

  /** @param {string} by @param {Palette} t */
  ink(by, t) {
    if (by === "model") return t["action-text"];
    return by === "human" ? t.emph : t["text-3"];
  }

  /** @param {PathBuilder} builder @param {string} dash */
  dashed(builder, dash) {
    return dash ? builder.dash_array(dash.split(" ").map(Number)) : builder;
  }

  /** @param {Edge} edge @param {number} scale @param {Palette} t */
  edge(edge, scale, t) {
    const style = edgeStyle(edge.by);
    const [x, y] = [edge.mid.x * scale + (edge.vertical ? 6 : 5), edge.mid.y * scale - 8];
    return div().absolute().inset_0()
      .when(edge.ungated, (el) => el.child(window.paint_path(build(PathBuilder.stroke(style.width + 6), edge.d, scale), Background.solid(t["danger-text"]).opacity(0.22)).absolute().inset_0()))
      .child(window.paint_path(build(this.dashed(PathBuilder.stroke(style.width), style.dash), edge.d, scale), this.ink(edge.by, t)).absolute().inset_0())
      .child(window.paint_path(arrowhead(edge.d, scale), this.ink(edge.by, t)).absolute().inset_0())
      .child(div().absolute().left(x).top(y).px(2).rounded(3).bg(t.bg).text_size(11).text_color(t["text-2"])
        .accessibility_label(`${edge.outcome}: ${edge.from} to ${edge.to}, ${word(BY_WORD, edge.by)}${edge.ungated ? ", reaches an irreversible step with no person's decision" : ""}`)
        .child(clip(edge.outcome, edge.vertical ? 10 : 14)));
  }

  /** @param {Node} node @param {number} scale @param {Palette} t */
  node(node, scale, t) {
    const [width, height] = [node.width * scale, node.height * scale];
    const box = div().absolute().left(node.x * scale).top(node.y * scale).w(width).h(height);
    if (node.terminal) {
      return box.rounded(height / 2).bg(t.raised).flex().items_center().justify_center()
        .font_family(this.font.mono).text_size(12).text_color(t["text-2"]).accessibility_label(node.label).child(node.label);
    }
    const irreversible = node.kind === "irreversible";
    const { d } = nodeShape(node.kind, width, height);
    const second = node.gate ? clip(node.gate, 24) : word(KIND_WORD, node.kind);
    return box.accessibility_label(`${node.label}: ${word(KIND_WORD, node.kind)}${node.gate ? `, ${node.gate}` : ""}`)
      .child(window.paint_path(build(PathBuilder.fill(), d, 1), this.fill(node.kind, t)).absolute().inset_0())
      .child(window.paint_path(build(PathBuilder.stroke(irreversible ? 2.5 : 1.5), d, 1), irreversible ? t["danger-text"] : t["control-line"]).absolute().inset_0())
      .child(v_flex().absolute().inset_0().items_center().justify_center().px(S["space-3"])
        .child(h_flex().gap(S["space-1"]).items_center().max_w_full()
          .when(irreversible, (el) => el.child(svg(ICON("lock")).size(12).flex_none().text_color(t["danger-text"])))
          .child(div().truncate().font_family(this.font.mono).text_size(12).font_weight(600).child(clip(node.label, 20))))
        .child(div().max_w_full().truncate().text_size(10).text_color(irreversible ? t["danger-text"] : t["text-3"]).child(second)));
  }

  /** @param {string} id @param {string} label @param {(cx: import("gpui-kit").Context) => void} action @param {Palette} t */
  secondary(id, label, action, t) {
    return Button.new(id).h(S["control-fine"] - 4).px(S["space-3"]).flex().flex_none().items_center().rounded(S["radius-card"]).border(1).border_color(t["control-line"])
      .bg(t.surface).text_color(t.text).text_size(S["type-small"][0]).font_weight(600).hover((s) => s.bg(t.raised))
      .focus((s) => s.border(2).border_color(t["action-text"])).on_click((_event, cx) => action(cx)).child(label);
  }
}
