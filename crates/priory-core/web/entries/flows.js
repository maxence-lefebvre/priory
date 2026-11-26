import { laterList } from "../legacy/frame.js";
import { Flows } from "../pages/Flows.jsx";

export const flows = {
  name: "workflow",
  Page: Flows,
  list: () => laterList("workflow"),
};
