import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { More } from "./More.jsx";
import { Place } from "./place.js";

const LAYOUT = { tabs: ["desk", "asks", "tasks", "runs", "inbox", "queue", "inbox"].map((panel) => ({ panel })) };

test("More lists the panels past the first four once each, then Settings and the sign-out", () => {
  const html = renderToStaticMarkup(<Place.Provider value={{ size: "compact", layout: LAYOUT }}><More /></Place.Provider>);
  assert.deepEqual([...html.matchAll(/<a href="([^"]+)">/g)].map(([, href]) => href), ["/inbox", "/queue", "/settings/appearance", "/settings/notifications"]);
  assert.match(html, /^<div class="flat"><div class="top"><h1>More<\/h1><\/div>/);
  assert.match(html, /<div class="signout"><button type="button" class="btn secondary">.*Sign out of this device<\/button><\/div>/);
});
