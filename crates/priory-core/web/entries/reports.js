import { later } from "../pages/Later.jsx";
import { Reports } from "../pages/Reports.jsx";

export const reports = {
  name: "reports",
  Page: Reports,
  List: later("reports"),
};
