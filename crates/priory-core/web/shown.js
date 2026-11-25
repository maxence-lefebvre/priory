// Whether this tab is on screen: a hidden tab reads nothing until it is shown.
import { store } from "./store.js";

export const shown = store(typeof document === "undefined" || !document.hidden);
