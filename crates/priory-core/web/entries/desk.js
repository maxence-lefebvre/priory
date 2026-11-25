import { deskClick, deskList, deskLive, deskPhone, deskView, seen } from "../legacy/desk.js";
import { current } from "../legacy/shared.js";

export const desk = {
  name: "desk",
  needs: ["asks", "queue", "tasks"],
  list: deskList,
  page: () => deskPhone(),
  older: { tasks: () => current() === "desk" && deskView() === "all" },
  click: deskClick,
  after: (host) => { if (host.querySelector("[data-since]")) seen(); },
  live: deskLive,
};
