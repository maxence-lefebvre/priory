import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClientProvider, QueryObserver } from "@tanstack/react-query";
import { shellOf, where } from "../frame.js";
import { client, moved } from "../queries.js";
import { setFailed } from "../session.js";
import { Frame } from "./Frame.jsx";
import { Icon } from "./Icon.jsx";
import { Linked } from "./Text.jsx";

const layout = { tabs: ["desk", "asks", "tasks", "runs", "queue", "inbox"].map((panel) => ({ panel })) };
const views = { factory: "shop", views: { default: { layout } } };
const draw = (t, element) => {
  const cache = client({ api: async () => ({}) });
  t.after(() => cache.clear());
  return renderToStaticMarkup(<QueryClientProvider client={cache}>{element}</QueryClientProvider>);
};
const frame = (t, href, size, open = 2) => draw(t, <Frame shell={shellOf(where(href), { size, list: "desk", layout })} size={size} views={views} open={open} onRetry={() => {}}><main /></Frame>);

test("a wide screen draws the top bar, the rail with every panel, then the page", (t) => {
  const html = frame(t, "/", "wide");
  assert.match(html, /^<header class="wtop"><span class="fac">shop<\/span><span class="conn ">live<\/span><span class="sp"><\/span><button type="button" class="addtop" data-action="add-open" aria-label="Add a task" aria-keyshortcuts="N" aria-haspopup="dialog">/);
  assert.match(html, /<div class="wide"><nav class="rail" aria-label="Panels"><a href="\/" aria-current="page">/);
  assert.match(html, /<span class="badge num" aria-label="2 open asks">2<\/span>/);
  assert.match(html, /<a href="\/inbox" aria-current="false">.*<a class="settings" href="\/settings\/appearance" aria-current="false">.*Settings<\/a><\/nav><main><\/main><\/div>$/);
});

test("a phone draws the page, then four tabs and More", (t) => {
  const html = frame(t, "/queue", "compact", 0);
  assert.match(html, /^<div class="flat"><main><\/main><\/div><nav class="tabbar" aria-label="Panels">/);
  assert.deepEqual([...html.matchAll(/href="([^"]+)" aria-current="(\w+)"/g)].map(([, href, current]) => `${href} ${current}`), ["/ false", "/asks false", "/tasks false", "/runs false", "/more page"]);
  assert.ok(!html.includes("badge"));
});

test("an ask takes the phone's whole screen", (t) => {
  assert.equal(frame(t, "/asks/4", "compact"), '<div class="flat"><main></main></div>');
});

test("a failed read shows the strip, with a Retry", (t) => {
  t.after(() => setFailed(null));
  setFailed("the connection dropped");
  const html = frame(t, "/", "compact");
  assert.match(html, /^<div class="stale" role="status"><svg class="ico s15".*<span>Reconnecting: the connection dropped. <\/span><button type="button" class="ghost" data-retry="true">Retry<\/button><\/div>/);
});

test("a page's own listing that fails says so, and the app stays live", async (t) => {
  const cache = client({ api: async () => { throw new Error("/api/queue answered 500"); } });
  t.after(() => cache.clear());
  cache.setQueryData(["queue"], { waiting: [] });
  t.after(new QueryObserver(cache, { queryKey: ["queue"], retry: false }).subscribe(() => {}));
  await moved(cache);
  const html = renderToStaticMarkup(<QueryClientProvider client={cache}><Frame shell={shellOf(where("/queue"), { size: "wide", list: "desk", layout })} size="wide" views={views} open={0} onRetry={() => {}} /></QueryClientProvider>);
  assert.match(html, /^<div class="stale" role="status"><svg.*<span>Could not read the queue again: \/api\/queue answered 500. This is its last read.<\/span><button type="button" class="ghost" data-retry="true">Retry<\/button><\/div>.*<span class="conn ">live<\/span>/);
});

test("an icon draws its strokes", () => {
  assert.equal(renderToStaticMarkup(<Icon name="plus" size="s20" />), '<svg class="ico s20" viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h14"></path><path d="M12 5v14"></path></svg>');
});

test("agent text stays text", (t) => {
  assert.equal(draw(t, <Linked text={'<img src=x onerror="a()"> https://example.com/1'} />), '&lt;img src=x onerror=&quot;a()&quot;&gt; <a class="autolink" href="https://example.com/1" rel="noopener noreferrer" target="_blank">https://example.com/1</a>');
});
