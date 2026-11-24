// The route React renders: the path and query of the address bar. go() and Back both move it.
import { flushSync } from "react-dom";
import { store } from "./store.js";

// A new object for each visit: a visit to the address already stored still draws.
const visit = () => ({ at: typeof location === "undefined" ? "/" : location.pathname + location.search });
export const route = store(visit());

// Draws the new page before it returns, so code after it finds the page's elements.
const move = () => flushSync(() => route.set(visit()));

export function go(path) {
  history.pushState(null, "", path);
  move();
}

export function followBack() {
  addEventListener("popstate", move);
  return () => removeEventListener("popstate", move);
}
