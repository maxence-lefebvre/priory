import { laterList } from "../legacy/frame.js";
import { Costs } from "../pages/Costs.jsx";

export const costs = {
  name: "costs",
  Page: Costs,
  list: () => laterList("costs"),
};
