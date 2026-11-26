import { AskDrawer, AskPage, AsksList, AsksPage, asksKey } from "../pages/Asks.jsx";
import { current, listSearch, selectedAsk } from "../legacy/shared.js";

export const asks = {
  name: "asks",
  List: AsksList,
  Page: AsksPage,
  Detail: AskPage,
  Drawer: AskDrawer,
  older: { asks: () => Boolean(selectedAsk()) || (current() === "asks" && ["answered", "all"].includes(new URLSearchParams(listSearch()).get("view"))) },
  key: asksKey,
};
