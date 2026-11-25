import { laterList } from "../legacy/frame.js";
import { acceptReport, dropPick, reportsClick, reportsInput, reportsPage, reportsSubmit } from "../legacy/reports.js";
import { current, html } from "../legacy/shared.js";

export const reports = {
  name: "reports",
  needs: ["report", "reports", "tasks"],
  page: (size) => (size === "compact" ? reportsPage(true) : html`<div class="list ">${reportsPage(false)}</div>`),
  list: () => laterList("reports"),
  kept: ["report", "reports"],
  fields: ["report-draft", "report-query"],
  click: reportsClick,
  submit: reportsSubmit,
  input: reportsInput,
  before: () => { if (current() !== "reports") dropPick(); },
  landed: acceptReport,
  left: dropPick,
};
