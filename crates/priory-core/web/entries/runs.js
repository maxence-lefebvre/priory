import { current, listView } from "../legacy/shared.js";
import { RunsList, RunsPage } from "../pages/Runs.jsx";
import { RUN_VIEWS } from "../runs.js";

export const runs = {
  name: "runs",
  List: RunsList,
  Page: RunsPage,
  older: { runs: () => current() === "runs" && listView(RUN_VIEWS) === "recent" },
};
