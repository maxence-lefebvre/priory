// What every Priory panel draws its copyable text with. gpui draws a div's text unselectable, so
// text a person may copy goes through a selectable TextView. Text inside a Button or a row that
// opens on a click stays a label, as in a desktop list: the row's detail shows it selectable.
import { TextView } from "gpui-base";
import { spansHtml } from "./priory.js";

/**
 * Text a person can select and copy, with its autolinks when given spans. It takes its font, size
 * and color from its parent, and wraps; it cannot truncate.
 * @param {string} id unique in the panel @param {string | { text: string, url?: string }[]} text
 */
export const selectable = (id, text) => TextView.html(id, spansHtml(typeof text === "string" ? [{ text }] : text)).selectable(true);
