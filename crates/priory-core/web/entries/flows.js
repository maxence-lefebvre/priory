import { Flows } from "../pages/Flows.jsx";
import { later } from "../pages/Later.jsx";

export const flows = {
  name: "workflow",
  Page: Flows,
  List: later("workflow"),
};
