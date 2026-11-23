import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { DOMParser, parse } from "./dom.js";
import { anchor, blockIn } from "../lib/page.js";

// The fixtures are scrubbed: octo/demo, #1, alice and src/app.js stand in for any real page.
const fixture = (name) => readFileSync(new URL(`fixtures/${name}`, import.meta.url), "utf8");
const OLD = fixture("old-pr.html");
const NEW = fixture("new-pr-overview.html");
const NEW_FILES = fixture("new-pr-files.html");
globalThis.DOMParser = DOMParser;
const BARE = '<html><body><main><h1>Fix a bug</h1><div class="comment-body"><p>Fix a bug.</p></div></main></body></html>';

test("on the old page, the strip's anchor is the pull request's tab row", () => {
  const found = anchor(parse(OLD));
  assert.equal(found.kind, "old");
  assert.ok(found.nav.matches(".tabnav-pr"));
  assert.ok(found.nav.querySelector('nav[aria-label="Pull request navigation"]'));
});

test("on the old page, the Guide reads the review guide's JSON from the description", () => {
  const json = blockIn(parse(OLD));
  assert.equal(JSON.parse(json).steps[0].file, "src/app.js");
});

test("on the new page, the strip's anchor is the tab row, and the strip goes before the React app", () => {
  for (const html of [NEW, NEW_FILES]) {
    const found = anchor(parse(html));
    assert.equal(found.kind, "new");
    assert.equal(found.nav.localName, "nav");
    assert.ok(found.nav.querySelector("#prs-files-anchor-tab"));
    assert.equal(found.at.localName, "react-app");
  }
});

test("on the new page, the Guide reads the review guide's JSON from the description on Overview", () => {
  assert.equal(JSON.parse(blockIn(parse(NEW))).steps[0].file, "src/app.js");
  assert.equal(blockIn(parse(NEW_FILES)), null);
});

test("where the new page renders the description in the browser only, the Guide reads it from the embedded data", () => {
  const doc = parse(NEW);
  const body = doc.querySelector(".markdown-body");
  const block = `<p>Fix a bug.</p><details><summary>Review guide</summary><pre><code>${body.querySelector("pre").textContent}</code></pre></details>`;
  body.remove();
  doc.querySelector('script[data-target="react-app.embeddedData"]').replaceChildren(JSON.stringify({ payload: { pullRequest: { body: { html: block } } } }));
  assert.equal(JSON.parse(blockIn(doc)).steps[0].file, "src/app.js");
});

test("a page with no tab row has no anchor, so the strip takes the fallback place", () => {
  assert.equal(anchor(parse(BARE)), null);
  assert.equal(blockIn(parse(BARE)), null);
});

// content.js runs as GitHub's page would run it: one document, its location and the page's fetch.
const observers = [];
let run = 0;
async function load(html, url) {
  observers.length = 0;
  const warnings = [];
  Object.assign(globalThis, {
    DOMParser,
    document: parse(html),
    location: new URL(url),
    MutationObserver: class {
      constructor(callback) {
        observers.push(callback);
      }
      observe() {}
    },
    fetch: async () => ({ ok: true, text: async () => html }),
    chrome: { runtime: { id: "x", getURL: (path) => new URL(`../${path}`, import.meta.url).href, onMessage: { addListener() {} } } },
  });
  const warn = console.warn;
  console.warn = (message) => warnings.push(message);
  await import(`../content.js?run=${(run += 1)}`);
  return { warnings, restore: () => { console.warn = warn; } };
}

const flush = () => new Promise((resolve) => setImmediate(resolve));
async function until(check, what) {
  for (let i = 0; i < 400; i += 1) {
    if (check()) return;
    await flush();
  }
  assert.fail(`timed out waiting for ${what}`);
}
const strips = () => document.querySelectorAll("iframe").filter((frame) => frame.src.includes("view=strip"));

test("content.js draws the strip above the old page's tab row, adds the Guide tab and redraws on soft navigation", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const page = await load(OLD, "https://github.com/octo/demo/pull/1");
  t.after(page.restore);
  const nav = () => document.querySelector('nav[aria-label="Pull request navigation"]');
  await until(() => nav().querySelector('a[href="#priory-guide"]'), "the Guide tab");
  assert.equal(strips().length, 1);
  assert.equal(strips()[0].dataset.at, "old");
  const row = document.querySelector(".tabnav-pr");
  assert.equal(row.parentElement.childNodes.indexOf(strips()[0]) + 1, row.parentElement.childNodes.indexOf(row));
  const guideTab = nav().querySelector('a[href="#priory-guide"]');
  assert.equal(guideTab.textContent, "Guide 1");
  assert.equal(guideTab.getAttribute("aria-current"), null);
  assert.equal(guideTab.classList.contains("selected"), false);

  // A soft navigation to another tab of the same pull request keeps the one strip.
  location.href = "https://github.com/octo/demo/pull/1/files";
  observers.forEach((callback) => callback());
  t.mock.timers.tick(150);
  await flush();
  assert.equal(strips().length, 1);

  // GitHub swaps the page for another pull request: the strip leaves with the old one and draws again.
  const next = parse(OLD.replaceAll("/octo/demo/pull/1", "/octo/demo/pull/2")).body;
  document.body.replaceChildren(...next.childNodes);
  location.href = "https://github.com/octo/demo/pull/2";
  observers.forEach((callback) => callback());
  t.mock.timers.tick(150);
  await until(() => strips().length === 1 && strips()[0].isConnected, "the strip on the next pull request");
  assert.equal(nav().querySelector("a").getAttribute("href"), "/octo/demo/pull/2");
  assert.equal(strips()[0].dataset.at, "old");
  assert.equal(page.warnings.length, 0);
});

test("content.js draws the strip above the new page's React app, adds the Guide tab to the row and keeps both through tabs and re-renders", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const page = await load(NEW, "https://github.com/octo/demo/pull/1");
  t.after(page.restore);
  const list = () => document.querySelector("nav ul");
  const guideItem = () => list().children.find((item) => item.querySelector('a[href="#priory-guide"]'));
  await until(() => guideItem(), "the Guide tab");
  const app = document.querySelector("react-app");
  assert.equal(strips().length, 1);
  assert.equal(strips()[0].dataset.at, "new");
  assert.equal(app.parentElement.childNodes.indexOf(strips()[0]) + 1, app.parentElement.childNodes.indexOf(app));
  assert.equal(list().children.at(-1), guideItem());
  const tab = guideItem().querySelector("a");
  assert.equal(tab.querySelector('[data-component="text"]').textContent, "Guide 1");
  for (const name of ["aria-current", "id", "data-dd-action-name", "tabindex"]) assert.equal(tab.getAttribute(name), null, name);

  // React moves to Files changed in place: one strip, and the row it redraws gets the Guide tab again.
  location.href = "https://github.com/octo/demo/pull/1/changes";
  const files = parse(NEW_FILES);
  document.querySelector('[data-component="PH_Navigation"]').replaceChildren(...files.querySelector('[data-component="PH_Navigation"]').childNodes);
  observers.forEach((callback) => callback());
  t.mock.timers.tick(150);
  await until(() => guideItem(), "the Guide tab on Files changed");
  assert.equal(strips().length, 1);
  assert.ok(strips()[0].isConnected);
  assert.equal(document.querySelector("#prs-files-anchor-tab").getAttribute("aria-current"), "page");
  assert.equal(page.warnings.length, 0);
});

test("content.js draws the strip when the new page opens on Files changed", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const page = await load(NEW_FILES, "https://github.com/octo/demo/pull/1/changes");
  t.after(page.restore);
  await until(() => strips().length === 1, "the strip");
  assert.equal(strips()[0].dataset.at, "new");
  assert.equal(strips()[0].parentElement, document.querySelector("react-app").parentElement);
});

test("content.js puts the strip at the top of a page with no tab row, warns once, and moves it when a row shows", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const page = await load(BARE, "https://github.com/octo/demo/pull/1");
  t.after(page.restore);
  await until(() => strips().length === 1, "the fallback strip");
  assert.equal(document.querySelector("main").childNodes[0], strips()[0]);
  assert.equal(strips()[0].dataset.at, "fallback");

  t.mock.timers.tick(3000);
  observers.forEach((callback) => callback());
  t.mock.timers.tick(150);
  t.mock.timers.tick(3000);
  await flush();
  assert.equal(page.warnings.length, 1);
  assert.match(page.warnings[0], /no tab row found on https:\/\/github\.com\/octo\/demo\/pull\/1/);

  const row = parse(OLD).querySelector(".tabnav-pr");
  document.querySelector("main").append(row);
  observers.forEach((callback) => callback());
  t.mock.timers.tick(150);
  await flush();
  assert.equal(strips().length, 1);
  assert.equal(strips()[0].dataset.at, "old");
  assert.equal(strips()[0].parentElement, document.querySelector("main"));
  assert.equal(row.parentElement.childNodes.indexOf(strips()[0]) + 1, row.parentElement.childNodes.indexOf(row));
});
