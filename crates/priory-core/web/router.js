// The navigation: one history that TanStack Router reads, how a page or a link moves it, and the list
// a detail was opened from. routes.jsx holds the routes.
import { createBrowserHistory, createMemoryHistory } from "@tanstack/react-router";
import { decoded } from "./pages.js";
import { START, arrive, entryOf, pathOf, searchOf, step } from "./frame.js";

// The router decodes each part of a path and throws on one that does not decode: `%` then stands for itself.
export function wellFormed(href) {
  const [path] = href.split(/[?#]/);
  return path.split("/").map((part) => (decoded(part) === part ? part.replaceAll("%", "%25") : part)).join("/") + href.slice(path.length);
}

if (typeof window !== "undefined") {
  const { pathname, search, hash } = window.location;
  const href = wellFormed(pathname + search + hash);
  if (href !== pathname + search + hash) window.history.replaceState(window.history.state, "", href);
}

export const history = typeof window === "undefined" ? createMemoryHistory() : createBrowserHistory();

// The search string is the URL's own text: the router's default would write each value as JSON.
export const parseSearch = (search) => Object.fromEntries(new URLSearchParams(search));
export const stringifySearch = (search) => {
  const text = new URLSearchParams(Object.entries(search).filter(([, value]) => value !== undefined)).toString();
  return text ? `?${text}` : "";
};

/** The address of a location of the router, as the address bar shows it. */
export const hrefOf = (location) => location.publicHref ?? location.href;

let trail = START;
let drawn = null;
let seen = { location: null, ready: false, place: null };

/**
 * Where the app stands at `location`: its address, its trail, and the address that replaces it when
 * the Desk opens an ask as the person arrives. The same location answers the same place, until the
 * Desk first has data.
 */
export function placeAt(location, desk, wide) {
  const ready = Boolean(desk);
  if (seen.location === location && seen.ready === ready) return seen.place;
  const href = hrefOf(location);
  trail = step(trail, href, location.state);
  const key = pathOf(href) + searchOf(href);
  const ask = ready && wide ? arrive(desk, { href, from: drawn, list: trail.list }) : null;
  const to = ask === null ? null : `/asks/${encodeURIComponent(ask)}`;
  if (to) trail = { list: "desk", listUrl: key, from: key };
  if (ready) drawn = to ?? key;
  seen = { location, ready, place: { href: to ?? href, trail, entry: location.state ?? {}, replace: to } };
  return seen.place;
}

export function go(href) {
  history.push(wellFormed(href), entryOf(trail, href));
  history.flush();
}

export function replace(href) {
  history.replace(wellFormed(href), entryOf(trail, href));
  history.flush();
}

const smooth = () => (matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth");

/** Scrolls to the element a hash names, if the page drew it. */
export const toHash = (hash, behavior = smooth()) => {
  const element = hash && document.getElementById(decoded(hash.slice(1)));
  element?.scrollIntoView({ behavior });
  return Boolean(element);
};

/** Follows a click on a link: to another page here, or to a section of this page. */
export function followLink(event) {
  const link = event.target.closest?.("a[href]");
  const modified = event.metaKey || event.ctrlKey || event.shiftKey || event.altKey;
  if (event.defaultPrevented || event.button !== 0 || modified || !link || link.origin !== window.location.origin || link.target) return;
  event.preventDefault();
  const inPage = link.pathname === window.location.pathname && link.search === window.location.search;
  if (inPage && link.hash) toHash(link.hash);
  else go(link.pathname + link.search + link.hash);
}
