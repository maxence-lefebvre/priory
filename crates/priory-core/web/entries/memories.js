import { disarmMemory, memoriesBefore, memoriesClick, memoriesPage } from "../legacy/memories.js";
import { html } from "../legacy/shared.js";
import { state } from "../legacy/state.js";

export const memories = {
  name: "memories",
  page: (size) => (size === "compact" ? memoriesPage() : html`<div class="list page">${memoriesPage()}</div>`),
  click: memoriesClick,
  before: memoriesBefore,
  left: disarmMemory,
  drain: (element) => (element.closest("[data-action=memory-act]") ? state.memoryArmed : undefined),
};
