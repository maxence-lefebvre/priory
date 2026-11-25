// The screen's layout, and a clock that repaints ages once a minute.
import { useSyncExternalStore } from "react";

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
const watchMinute = (changed) => {
  const timer = setInterval(changed, 5000);
  return () => clearInterval(timer);
};

/** Redraws the component once a minute, since its ages read in minutes. */
export const useMinute = () => useSyncExternalStore(watchMinute, minute, minute);
