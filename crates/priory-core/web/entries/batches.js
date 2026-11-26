import { batchBefore, batchClick, batchPage, closePlanEdit, drawLanes } from "../legacy/batch.js";
import { html } from "../legacy/shared.js";
import { state } from "../legacy/state.js";

export const batches = {
  name: "batches",
  detailNeeds: ["tasks"],
  detail: (id, size) => (size === "compact" ? batchPage(id) : html`<div class="list page">${batchPage(id)}</div>`),
  click: batchClick,
  before: batchBefore,
  after: drawLanes,
  landed: () => { if (state.batch) state.batch.stale = true; },
  left: closePlanEdit,
};
