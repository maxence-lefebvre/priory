import { test } from "node:test";
import assert from "node:assert/strict";
import { followLink, go, history, parseSearch, placeAt, stringifySearch } from "./router.js";

test("a search string goes round as its own text, never as JSON", () => {
  for (const search of ["?view=needs", "?view=all&band=now", "?view=42", "?q=true", "?risk=high&runner=desk-1", ""]) {
    assert.equal(stringifySearch(parseSearch(search)), search);
  }
});

const ask = (id) => ({ kind: "ask", id, what: "?" });

test("an ask opened from a list remembers the list, and Back to it restores the list", () => {
  placeAt({ href: "/asks?view=answered", state: {} }, { items: [] }, false);
  go("/asks/7");
  assert.equal(history.location.href, "/asks/7");
  assert.deepEqual([history.location.state.list, history.location.state.listUrl], ["asks", "/asks?view=answered"]);
  const entry = history.location.state;
  placeAt({ href: "/?view=running", state: {} }, { items: [] }, false);
  const back = placeAt({ href: "/asks/7", state: entry }, { items: [] }, false);
  assert.deepEqual([back.trail.list, back.trail.listUrl], ["asks", "/asks?view=answered"]);
});

test("a task opened from a list remembers it for its back link, and Back and forward move the address", () => {
  placeAt({ href: "/queue?view=held", state: {} }, { items: [] }, false);
  go("/queue?view=held");
  go("/tasks/3");
  assert.equal(history.location.state.from, "/queue?view=held");
  history.back();
  assert.equal(history.location.href, "/queue?view=held");
  history.forward();
  assert.equal(history.location.href, "/tasks/3");
});

test("a wide Desk opens its first ask as the person arrives, once", () => {
  const desk = { items: [ask(7), ask(8)] };
  const arrived = { href: "/", state: {} };
  assert.equal(placeAt(arrived, undefined, true).replace, null, "nothing opens before the desk is read");
  const place = placeAt(arrived, desk, true);
  assert.deepEqual([place.href, place.replace, place.trail.listUrl], ["/asks/7", "/asks/7", "/"]);
  assert.equal(placeAt(arrived, { items: [ask(9)] }, true), place, "a redraw opens no other ask");
  assert.equal(placeAt({ href: "/", state: {} }, desk, true).replace, null, "closing that ask leaves the drawer empty");
  assert.equal(placeAt({ href: "/?view=running", state: {} }, desk, true).replace, null);
});

test("a narrow Desk opens none", () => {
  placeAt({ href: "/runs", state: {} }, { items: [] }, false);
  assert.equal(placeAt({ href: "/", state: {} }, { items: [ask(7)] }, false).replace, null);
});

// A click on a link to `href`, from `/tasks/1`.
function click(t, href, event = {}) {
  const at = new URL("http://priory/tasks/1");
  const to = new URL(href, at);
  const scrolled = [];
  Object.assign(globalThis, {
    window: { location: at },
    matchMedia: () => ({ matches: true }),
    document: { getElementById: (id) => (id === "gone" ? null : { scrollIntoView: (options) => scrolled.push([id, options.behavior]) }) },
  });
  t.after(() => { for (const name of ["window", "matchMedia", "document"]) delete globalThis[name]; });
  let prevented = false;
  const link = { origin: to.origin, pathname: to.pathname, search: to.search, hash: to.hash, target: event.target ?? "" };
  followLink({ button: 0, defaultPrevented: false, ...event, target: { closest: () => link }, preventDefault: () => { prevented = true; } });
  return { prevented, scrolled, at: history.location.href };
}

test("a link to a page goes there, with its search and hash", (t) => {
  assert.deepEqual(click(t, "/inbox?view=unrouted#dropped"), { prevented: true, scrolled: [], at: "/inbox?view=unrouted#dropped" });
});

test("a link to a section of the page scrolls to it, and keeps the address", (t) => {
  go("/tasks/1");
  assert.deepEqual(click(t, "#say"), { prevented: true, scrolled: [["say", "auto"]], at: "/tasks/1" });
  assert.deepEqual(click(t, "#gone"), { prevented: true, scrolled: [], at: "/tasks/1" });
});

test("a modified click, another button, another target or another site is the browser's", (t) => {
  go("/tasks/1");
  for (const event of [{ metaKey: true }, { ctrlKey: true }, { shiftKey: true }, { altKey: true }, { button: 1 }, { target: "_blank" }, { defaultPrevented: true }]) {
    assert.deepEqual(click(t, "/runs", event), { prevented: false, scrolled: [], at: "/tasks/1" }, JSON.stringify(event));
  }
  assert.equal(click(t, "https://example.com/runs").prevented, false);
});
