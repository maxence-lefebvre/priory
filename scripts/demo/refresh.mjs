// Checks that a refresh keeps the page drawn: it changes the demo's data while a page is open, samples
// the page every animation frame, and fails on a frame that lost a section, the Say field's element,
// focus, value or caret, or that drew "Loading task". It also counts what each page reads once the
// revision moves, and checks the hidden tab, Back, a superseded refresh and a draw that throws.
// Usage: node scripts/demo/refresh.mjs DIR [CHROMIUM]
// DIR is the folder scripts/demo/serve.sh serves. CHROMIUM defaults to $CHROMIUM, then `chromium`.
// With $WEB set to crates/priory-core/web, the page loads its bundle and app.css in place of the server's,
// so a web change needs no Rust build to check.
// It drives Chromium over the DevTools protocol with Node's own WebSocket, so it needs no package, only the `sqlite3` command.
import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [dir, chromium = process.env.CHROMIUM || "chromium"] = process.argv.slice(2);
if (!dir) throw new Error("usage: refresh.mjs DIR [CHROMIUM]");
const BASE = "http://127.0.0.1:7391";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const profile = mkdtempSync(join(tmpdir(), "priory-refresh-"));
const browser = spawn(chromium, ["--headless=new", "--no-sandbox", "--remote-debugging-port=0", `--user-data-dir=${profile}`, "about:blank"], { stdio: ["ignore", "ignore", "pipe"] });
const address = await new Promise((resolve, reject) => {
  let seen = "";
  browser.stderr.on("data", (chunk) => {
    seen += chunk;
    const found = seen.match(/DevTools listening on (ws:\/\/\S+)/);
    if (found) resolve(found[1]);
  });
  browser.on("exit", () => reject(new Error(`chromium exited: ${seen}`)));
});

const socket = new WebSocket(address);
await new Promise((resolve) => socket.addEventListener("open", resolve, { once: true }));
let next = 0;
const waiting = new Map();
const web = process.env.WEB;
const served = { "/app.js": ["dist/app.js", "text/javascript"], "/app.css": ["app.css", "text/css"] };
socket.addEventListener("message", ({ data }) => {
  const message = JSON.parse(data);
  if (message.method === "Fetch.requestPaused") {
    const [file, type] = served[new URL(message.params.request.url).pathname];
    page("Fetch.fulfillRequest", { requestId: message.params.requestId, responseCode: 200, responseHeaders: [{ name: "Content-Type", value: type }], body: readFileSync(join(web, file)).toString("base64") });
  }
  if (waiting.has(message.id)) {
    const { resolve, reject } = waiting.get(message.id);
    waiting.delete(message.id);
    message.error ? reject(new Error(message.error.message)) : resolve(message.result);
  }
});
const send = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
  const id = ++next;
  waiting.set(id, { resolve, reject });
  socket.send(JSON.stringify({ id, method, params, sessionId }));
});

const { targetId } = await send("Target.createTarget", { url: "about:blank" });
const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
const page = (method, params) => send(method, params, sessionId);
const evaluate = async (expression) => {
  const { result, exceptionDetails } = await page("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (exceptionDetails) throw new Error(exceptionDetails.exception?.description || exceptionDetails.text);
  return result.value;
};
const until = async (expression, what) => {
  for (let i = 0; i < 100; i++) {
    // An element React drew holds its whole tree, which DevTools cannot send.
    if (await evaluate(`Boolean(${expression})`)) return;
    await sleep(100);
  }
  throw new Error(`timed out waiting for ${what}`);
};
await page("Page.enable");
if (web) {
  await page("Network.enable");
  await page("Network.setBypassServiceWorker", { bypass: true });
  await page("Fetch.enable", { patterns: Object.keys(served).map((path) => ({ urlPattern: `*${path}` })) });
}
await page("Emulation.setDeviceMetricsOverride", { width: 1440, height: 900, deviceScaleFactor: 1, mobile: false });
// `reads` holds the paths the page read, and `hidden` is what the page says of its tab.
await page("Page.addScriptToEvaluateOnNewDocument", { source: `
  localStorage.setItem("priory.token", "demo-token");
  globalThis.reads = []; globalThis.hidden = false; globalThis.landed = 0;
  const read = fetch;
  globalThis.fetch = async (path, options) => {
    const counted = String(path).startsWith("/api/") && !String(path).startsWith("/api/events") && options?.method !== "POST";
    if (counted) reads.push(path);
    try { return await read(path, options); } finally { if (counted) landed++; }
  };
  Object.defineProperty(document, "hidden", { get: () => globalThis.hidden });
` });

// Changes a task the page does not show, so the listings change and the page reads again.
let changes = 0;
const change = () => execFileSync("sqlite3", [join(dir, "state.db"), `UPDATE tasks SET tldr = 'Probe ${++changes}' WHERE id = 2; UPDATE revision SET value = value + 1;`]);

// Samples every frame until the page read again and a second passed, then returns what it saw.
const SAMPLE = (probe) => `new Promise((done) => {
  const frames = [];
  const start = performance.now();
  const before = landed;
  let reread = 0;
  const tick = () => {
    if (landed > before) reread = reread || performance.now();
    frames.push((${probe})());
    if (reread && performance.now() - reread > 1000 || performance.now() - start > 30000) return done({ reread: Boolean(reread), frames });
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
})`;

const results = [];
function report(ok, name, detail = "") {
  results.push(ok);
  console.log(`${ok ? "ok" : "FAILED"} ${name}${detail ? `: ${detail}` : ""}`);
}

async function open(path, ready) {
  await page("Page.navigate", { url: BASE + path });
  await until(ready, `${path} to draw`);
}

async function check(name, path, ready, probe, bad, prepare = async () => {}) {
  await open(path, ready);
  // The page has read what it shows, and waits on the next event.
  await sleep(1500);
  await prepare();
  const sampled = evaluate(SAMPLE(probe));
  await sleep(300);
  change();
  const { reread, frames } = await sampled;
  const wrong = frames.map(bad).filter(Boolean);
  report(reread && wrong.length === 0, name, `${frames.length} frames${reread ? "" : ", the page never read again"}${wrong.length ? `, ${wrong.length} wrong, first: ${wrong[0]}` : ""}`);
}

// The ask drawer keeps the task's sections it drew before the refresh.
const sections = await (async () => {
  await open("/asks/1", 'document.querySelector(".drawer #trace")');
  return evaluate('["brief", "inbox", "trace"].filter((id) => document.querySelector(".drawer #" + id))');
})();
await check("the ask drawer keeps its sections", "/asks/1", 'document.querySelector(".drawer #trace")',
  `() => ${JSON.stringify(sections)}.filter((id) => !document.querySelector(".drawer #" + id))`,
  (missing) => (missing.length ? `no ${missing.join(", ")}` : null));

// The task page keeps the very Say field, focused, with its text, selection and scroll, and never says it is loading.
const SAY = `() => { const say = document.getElementById("say"); return { same: say === globalThis.saySeen, focused: document.activeElement === say, value: say?.value, from: say?.selectionStart, to: say?.selectionEnd, scroll: document.querySelector("#app .scroll").scrollTop, loading: document.getElementById("app").innerText.includes("Loading task") }; }`;
const wrongSay = (frame) => (!frame.same ? "another Say element" : !frame.focused ? "focus lost" : frame.value !== "hello" ? `text ${JSON.stringify(frame.value)}`
  : frame.from !== 2 || frame.to !== 4 ? `selection ${frame.from} to ${frame.to}` : frame.scroll !== 40 ? `scroll ${frame.scroll}` : frame.loading ? "Loading task drawn" : null);
const typeSay = async () => {
  await evaluate('globalThis.saySeen = document.getElementById("say"); saySeen.focus()');
  await page("Input.insertText", { text: "hello" });
  await evaluate('saySeen.setSelectionRange(2, 4); document.querySelector("#app .scroll").scrollTop = 40');
  await sleep(100);
};
await check("the Say field keeps its element, focus, text and selection", "/tasks/1", 'document.getElementById("say")', SAY, wrongSay, typeSay);

// And through a change of the task it shows.
await open("/tasks/1", 'document.getElementById("say")');
await sleep(1500);
await typeSay();
execFileSync("sqlite3", [join(dir, "state.db"), "UPDATE tasks SET tldr = 'A new title' WHERE id = 1; UPDATE revision SET value = value + 1;"]);
await until(`document.querySelector("h1.question").textContent === "A new title"`, "the task's new title");
const after = wrongSay(await evaluate(`(${SAY})()`));
report(!after, "the Say field keeps them as its task changes on screen", after ?? "");
await evaluate(`document.querySelector('a.back').click()`);
await until('document.querySelector("#app h1")?.textContent === "Tasks"', "the Tasks list");
await evaluate("history.back()");
await until('document.getElementById("say")', "Back to the task");
report((await evaluate('document.getElementById("say").value')) === "hello", "the Say draft outlives its page");

// What each page reads once the revision moves.
// Each page's own file says it, so two ports touch no common line.
const folder = new URL("./pages/", import.meta.url);
const pages = await Promise.all(readdirSync(folder).sort().map((file) => import(new URL(file, folder))));
for (const [path, wanted] of pages.flatMap(({ reads }) => reads)) {
  await open(path, 'document.querySelector("#app nav")');
  await sleep(2000);
  await evaluate("reads.length = 0");
  change();
  // The pages left before may still hold the server's long polls, so this page's may come 5 s late.
  await until("reads.length", `${path} to read again`);
  await sleep(1500);
  const found = (await evaluate("reads")).sort().join(" ");
  report(found === wanted, `${path} reads only what it shows`, found);
}

// A hidden tab reads nothing, and reads once when it is shown.
const TASKS = pages.flatMap(({ reads }) => reads).find(([path]) => path === "/tasks")[1];
await open("/tasks", 'document.querySelector("#app nav")');
await sleep(2000);
const TAB = (off) => `globalThis.hidden = ${off}; document.dispatchEvent(new Event("visibilitychange"))`;
await evaluate(`reads.length = 0; ${TAB(true)}`);
change();
await sleep(2500);
const unseen = (await evaluate("reads")).length;
await evaluate(TAB(false));
await sleep(1500);
const seen = (await evaluate("reads")).sort().join(" ");
report(unseen === 0 && seen === TASKS, "a hidden tab reads nothing, and once when it is shown", `${unseen}, then ${seen}`);

// The page at PATH, after SCRIPT ran in it, must say true.
async function holds(name, path, ready, script) {
  await open(path, ready);
  await sleep(2000);
  report((await evaluate(script)) === true, name);
}
const COUNT = `let draws = 0; const inner = Object.getOwnPropertyDescriptor(Element.prototype, "innerHTML");
  Object.defineProperty(Element.prototype, "innerHTML", { ...inner, set(text) { if (this.className === "legacy") { draws++; if (globalThis.failDraw) { globalThis.failDraw = false; throw new Error("a draw that throws"); } } inner.set.call(this, text); } });`;
const SETTLE = (ms) => `await new Promise((done) => setTimeout(done, ${ms}));`;
const ROW = 'document.querySelector(".row")';
const TITLE = (name) => `document.querySelector("#app h1")?.textContent === "${name}"`;

await holds("Back and forward draw the page of their address", "/queue", TITLE("Queue"),
  `(async () => { document.querySelector('a[href="/runs"]').click(); ${SETTLE(500)} const there = ${TITLE("Runs")} && location.pathname === "/runs";
    history.back(); ${SETTLE(500)} const back = ${TITLE("Queue")}; history.forward(); ${SETTLE(500)} return there && back && ${TITLE("Runs")}; })()`);
await holds("a search string stays as it is written", "/tasks?view=ended&band=now", TITLE("Tasks"),
  'location.href.endsWith("/tasks?view=ended&band=now")');
// A legacy page draws with innerHTML; this goes with the last one.
await holds("a legacy draw that throws keeps the page", "/", ROW,
  `(async () => { ${COUNT} globalThis.failDraw = true; document.querySelector('a[href="/runs"]').click(); ${SETTLE(800)} return globalThis.failDraw === false && document.getElementById("app")?.innerText.length > 100; })()`);
await holds("a superseded refresh never draws", "/", ROW,
  `(async () => {
    const read = fetch; let hold = true;
    globalThis.fetch = async (path, options) => {
      const late = path === "/api/desk" && hold;
      if (late) hold = false;
      const reply = await read(path, { ...options, signal: undefined });
      if (!late) return reply;
      const body = { ...(await reply.json()), paused: "SUPERSEDED since 2026-01-01T00:00:00Z" };
      await new Promise((done) => setTimeout(done, 1500));
      return new Response(JSON.stringify(body), { status: 200 });
    };
    let seen = false;
    new MutationObserver(() => { seen ||= document.getElementById("app").innerText.includes("SUPERSEDED"); }).observe(document.getElementById("app"), { childList: true, subtree: true });
    const again = () => document.dispatchEvent(new Event("visibilitychange"));
    again(); ${SETTLE(300)} again(); ${SETTLE(3000)}
    return !hold && !seen;
  })()`);

// The list and the drawer draw apart, in one pass.
await holds("a list keeps its scroll while the drawer's ask changes", "/asks/1", `document.querySelector('.drawer [aria-label="Next ask"][href]')`,
  `(async () => { const list = () => document.querySelector(".list .scroll"); const next = document.querySelector('.drawer [aria-label="Next ask"]'); const to = next.getAttribute("href");
    list().scrollTop = 60; document.dispatchEvent(new Event("visibilitychange")); ${SETTLE(1500)} const kept = list().scrollTop === 60; next.click(); ${SETTLE(800)}
    return kept && location.pathname === to && document.querySelectorAll("#app .legacy").length === 2; })()`);
await holds("the drawer keeps its scroll through a refresh of the list", "/asks/1", 'document.querySelector(".drawer #trace")',
  `(async () => { const part = () => document.querySelector(".drawer .scroll"); part().scrollTop = 60; const at = part().scrollTop; document.dispatchEvent(new Event("visibilitychange")); ${SETTLE(1500)} return at > 0 && part().scrollTop === at; })()`);

for (const { check: own } of pages) if (own) await own({ open, holds, page, evaluate, until, sleep, change, report });

socket.close();
browser.kill();
await sleep(200);
rmSync(profile, { recursive: true, force: true });
process.exit(results.every(Boolean) ? 0 : 1);
