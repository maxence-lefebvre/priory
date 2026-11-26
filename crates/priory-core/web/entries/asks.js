import { answerSheet, askDetail, askView, asksBefore, asksBody, asksClick, asksInput, asksKey, asksLeft, asksPhone, asksSubmit, drawer, renderSheet } from "../legacy/asks.js";
import { keyHints, rowsList } from "../legacy/frame.js";
import { current, selectedAsk } from "../legacy/shared.js";
import { state } from "../legacy/state.js";

export const asks = {
  name: "asks",
  needs: ["asks"],
  detailNeeds: ["asks", "tasks"],
  list: (wide) => rowsList("Asks", asksBody(wide), { hints: wide ? keyHints() : "" }),
  page: asksPhone,
  detail: (id) => askDetail(id, false),
  drawer,
  fields: ["draft"],
  older: { asks: () => Boolean(selectedAsk()) || (current() === "asks" && ["answered", "all"].includes(askView())) },
  click: asksClick,
  submit: asksSubmit,
  input: asksInput,
  key: asksKey,
  sheet: () => { if (answerSheet().open && state.data) renderSheet(); },
  before: asksBefore,
  drain: () => state.armed,
  landed: () => { if (state.task) state.task.stale = true; },
  left: asksLeft,
};
