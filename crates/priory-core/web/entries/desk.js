import { DeskList, DeskPage, deskViewOf } from "../pages/Desk.jsx";
import { current, listSearch } from "../legacy/shared.js";

export const desk = {
  name: "desk",
  List: DeskList,
  Page: DeskPage,
  older: { tasks: () => current() === "desk" && deskViewOf(listSearch()) === "all" },
};
