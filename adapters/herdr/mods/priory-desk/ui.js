// What every Priory panel draws its copyable text and its state tiles with. gpui draws a div's text
// unselectable, so text a person may copy goes through a selectable TextView. Text inside a Button or
// a row that opens on a click stays a label, as in a desktop list: the row's detail shows it selectable.
import { div, svg } from "gpui-kit";
import { h_flex, TextView } from "gpui-base";
import { markdownHtml, spansHtml } from "./priory.js";
import { STRUCTURE as S } from "./tokens.js";

/**
 * Text a person can select and copy, with its autolinks when given spans. It takes its font, size
 * and color from its parent, and wraps; it cannot truncate.
 * @param {string} id unique in the panel @param {string | { text: string, url?: string }[]} text
 */
export const selectable = (id, text) => TextView.html(id, spansHtml(typeof text === "string" ? [{ text }] : text)).selectable(true);

/**
 * Agent Markdown a person can select and copy: DESIGN.md's subset, with the factory's autolinks.
 * @param {string} id unique in the panel @param {string} text @param {{ pattern: string, url: string }[]} links
 */
export const selectableMarkdown = (id, text, links) => TextView.html(id, markdownHtml(text, links, null)).selectable(true);

// DESIGN.md, State tile: the text color each role's icon and word share, by the role's fill token.
const ROLE_TEXT = /** @type {Record<string, string>} */ ({
  wait: "count", "tile-live": "action-text", "tile-warn": "warn-text", "tile-danger": "danger-text",
  "tile-ok": "ok-text", "tile-hold": "text-2", "tile-muted": "text-3",
});

/**
 * A state tile: a stroke icon in its role's color, centred in its slot, with no fill and no edge.
 * A null icon draws the live dot of a running task or run.
 * @param {string} state its word, which a screen reader reads @param {string | null} icon the icon's path
 * @param {string} role the role's fill token @param {Record<string, import("gpui-kit").Color>} t
 * @param {"tile" | "small"} [size] a row's tile, or the small tile of history rows and context lines
 */
export function stateTile(state, icon, role, t, size = "tile") {
  const color = t[ROLE_TEXT[role] ?? "text-3"];
  return h_flex().size(size === "tile" ? S["pane-tile"] : 18).flex_none().items_center().justify_center().accessibility_label(state)
    .child(icon ? svg(icon).size(size === "tile" ? 17 : 15).text_color(color) : div().size(8).rounded(S["radius-full"]).bg(color));
}
