import { laterList } from "../legacy/frame.js";
import { Reports } from "../pages/Reports.jsx";

export const reports = {
  name: "reports",
  Page: Reports,
  list: () => laterList("reports"),
};
