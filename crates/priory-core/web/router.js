// The visit React renders; the pages read the address bar. go() and Back both make one.
import { flushSync } from "react-dom";
import { store } from "./store.js";

// A new object for each visit: a visit to the address already shown still draws.
export const route = store({});

// Draws the new page before it returns, so code after it finds the page's elements.
const move = () => flushSync(() => route.set({}));

export function go(path) {
  history.pushState(null, "", path);
  move();
}

export function followBack() {
  addEventListener("popstate", move);
  return () => removeEventListener("popstate", move);
}
