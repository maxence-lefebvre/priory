import { flowsClick, flowsPage } from "../legacy/flows.js";
import { laterList } from "../legacy/frame.js";
import { html } from "../legacy/shared.js";
import { state } from "../legacy/state.js";

export const flows = {
  name: "workflow",
  page: (size) => (size === "compact" ? flowsPage(true) : html`<div class="list ">${flowsPage(false)}</div>`),
  list: () => laterList("workflow"),
  kept: ["flows"],
  click: flowsClick,
  signedOut: () => { state.flowsError = null; },
};
