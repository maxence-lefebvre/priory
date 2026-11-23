// The route React renders: the path and query of the address bar. go() and Back both move it.
import { flushSync } from "react-dom";
import { store } from "./store.js";

const where = () => location.pathname + location.search;
export const route = store(typeof location === "undefined" ? "/" : where());

// Draws the new page before go() returns, so code after it finds the page's elements.
export function go(path) {
  history.pushState(null, "", path);
  flushSync(() => route.set(where()));
}

export function followBack() {
  const back = () => route.set(where());
  addEventListener("popstate", back);
  return () => removeEventListener("popstate", back);
}
