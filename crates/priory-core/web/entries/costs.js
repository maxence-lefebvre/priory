import { costsChange, costsClick, costsDrawn, costsForget, costsPage, loadCosts } from "../legacy/costs.js";
import { laterList } from "../legacy/frame.js";
import { current, html } from "../legacy/shared.js";

export const costs = {
  name: "costs",
  page: (size) => (size === "compact" ? costsPage(true) : html`<div class="list ">${costsPage(false)}</div>`),
  list: () => laterList("costs"),
  kept: ["spend"],
  click: costsClick,
  change: costsChange,
  after: costsDrawn,
  landed: () => { if (current() === "costs") loadCosts(); },
  signedOut: costsForget,
};
