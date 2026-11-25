import { rowsList, rowsPage } from "../legacy/frame.js";
import { runsBody } from "../legacy/runs.js";
import { current, listView } from "../legacy/shared.js";
import { RUN_VIEWS } from "../runs.js";

export const runs = {
  name: "runs",
  needs: ["runs"],
  list: (wide) => rowsList("Runs", runsBody(wide)),
  page: () => rowsPage("Runs", runsBody(false)),
  older: { runs: () => current() === "runs" && listView(RUN_VIEWS) === "recent" },
};
