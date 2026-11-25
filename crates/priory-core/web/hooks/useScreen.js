// The screen's layout, and a clock that repaints ages once a minute.
import { useSyncExternalStore } from "react";
import { shown } from "../shown.js";

const WIDE = "(min-width: 1024px)";
const MEDIUM = "(min-width: 600px)";

const size = () => (matchMedia(WIDE).matches ? "wide" : matchMedia(MEDIUM).matches ? "medium" : "compact");
const watchSize = (changed) => {
  const queries = [WIDE, MEDIUM].map((query) => matchMedia(query));
  queries.forEach((query) => query.addEventListener("change", changed));
  return () => queries.forEach((query) => query.removeEventListener("change", changed));
};

/** "compact", "medium" or "wide". */
export const useSize = () => useSyncExternalStore(watchSize, size, () => "compact");

const minute = () => Math.floor(Date.now() / 60000);
// One clock for every component that shows an age, stopped while the tab is hidden.
const clocks = new Set();
let timer = null;
const tick = () => clocks.forEach((changed) => changed());
function turn() {
  const on = clocks.size > 0 && shown.get();
  if (on) timer ??= setInterval(tick, 5000);
  else timer = clearInterval(timer) ?? null;
}
shown.subscribe(() => {
  turn();
  if (shown.get()) tick();
});
const watchMinute = (changed) => {
  clocks.add(changed);
  turn();
  return () => {
    clocks.delete(changed);
    turn();
  };
};

/** Redraws the component once a minute, since its ages read in minutes. */
export const useMinute = () => useSyncExternalStore(watchMinute, minute, minute);
