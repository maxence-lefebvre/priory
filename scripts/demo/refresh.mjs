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
// A check that throws would leave the browser polling the server, which holds only four waits.
process.on("exit", () => browser.kill());
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
  // What a check does to the requests: a view's layout, paths that fail, that answer late or at another revision, and the answer of a POST.
  globalThis.hook = { fails: [], slow: {}, revision: {}, ...JSON.parse(sessionStorage.getItem("hook") || "{}") };
  if (hook.cold) for (const key of Object.keys(localStorage)) if (key.startsWith("priory.snapshot")) localStorage.removeItem(key);
  globalThis.posts = []; globalThis.polls = 0; globalThis.errors = []; globalThis.kept = 0; globalThis.clocks = new Set();
  addEventListener("error", (event) => errors.push(event.message));
  const { setInterval: every, clearInterval: stop } = globalThis;
  globalThis.setInterval = (run, ms, ...rest) => { const id = every(run, ms, ...rest); if (ms === 5000) clocks.add(id); return id; };
  globalThis.clearInterval = (id) => { clocks.delete(id); stop(id); };
  const setItem = Storage.prototype.setItem;
  Storage.prototype.setItem = function (key, value) { if (key.startsWith("priory.snapshot")) kept++; return setItem.call(this, key, value); };
  const read = fetch;
  globalThis.fetch = async (path, options) => {
    if (path === "/api/views" && hook.layout) {
      const views = await (await read(path, options)).json();
      views.views.default.layout = hook.layout;
      return new Response(JSON.stringify(views));
    }
    if (options?.method === "POST" && hook.reply && path !== "/api/seen") { posts.push([path, JSON.parse(options.body)]); return new Response(JSON.stringify(hook.reply), { status: 409 }); }
    const counted = String(path).startsWith("/api/") && !String(path).startsWith("/api/events") && options?.method !== "POST";
    if (counted) reads.push(path);
    if (String(path).startsWith("/api/events")) polls++;
    if (hook.slow[path]) await new Promise((done) => setTimeout(done, hook.slow[path]));
    if (hook.fails.includes(path)) return new Response("{}", { status: 500 });
    const named = hook.revision[path];
    if (named) return read(path, options).then(async (reply) => new Response(JSON.stringify({ ...(await reply.json()), revision: named }))).finally(() => { landed++; });
    const late = hook.late === path && await read(path, options);
    if (late) { hook.late = null; await new Promise((done) => setTimeout(done, 4000)); return late; }
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
await holds("a list keeps its scroll while the drawer's ask changes", "/asks/1", `location.pathname === "/asks/1" && document.querySelector('.drawer [aria-label="Next ask"][href]')`,
  `(async () => { const list = () => document.querySelector(".list .scroll"); const next = () => document.querySelector('.drawer [aria-label="Next ask"]'); const to = next().getAttribute("href");
    list().scrollTop = 60; document.dispatchEvent(new Event("visibilitychange")); ${SETTLE(1500)} const kept = list().scrollTop === 60; next().click(); ${SETTLE(800)}
    return kept && location.pathname === to && document.querySelectorAll("#app .legacy").length === 2; })()`);
await holds("the drawer keeps its scroll through a refresh of the list", "/asks/1", 'location.pathname === "/asks/1" && document.querySelector(".drawer #trace")',
  `(async () => { const part = () => document.querySelector(".drawer .scroll"); part().scrollTop = 60; const at = part().scrollTop; document.dispatchEvent(new Event("visibilitychange")); ${SETTLE(1500)} return at > 0 && part().scrollTop === at; })()`);

// Each check opens a page with the requests hooked, and says true, or what it found instead.
async function hooked(name, path, ready, hook, look) {
  let found;
  try {
    // A blank page first: an open page would keep its snapshot again as it leaves.
    await evaluate(`sessionStorage.setItem("hook", ${JSON.stringify(JSON.stringify(hook))})`);
    await open(path, ready);
    found = await look();
  } catch (error) {
    found = error.message;
  }
  report(found === true, name, found === true ? "" : JSON.stringify(found));
  await evaluate('sessionStorage.removeItem("hook")').catch(() => {});
}
const screen = (width, height) => page("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: width < 1000 });
const key = (name) => evaluate(`document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "${name}", bubbles: true, cancelable: true }))`);
const refused = { error: "refused by the check" };
const OPTION = 'document.querySelector(".drawer .opt[aria-disabled=false]")';
const SAYS = 'document.getElementById("say")';

await screen(390, 844);
await hooked("a next ask opens at its question with the bar hidden, and two taps where the bar was answer nothing", "/asks/1", 'document.querySelector("#app .opts")', { reply: refused }, async () => {
  await sleep(1500);
  await evaluate('document.querySelector("#app .scroll").scrollTop = 600');
  await until('document.querySelector("#app .abar")?.hidden === false', "the answer bar");
  const found = await evaluate(`(async () => {
    const before = document.querySelector("#app .scroll"), box = document.getElementById("bar-answer").getBoundingClientRect();
    document.querySelector('[aria-label="Next ask"]').click();
    while (document.querySelector("#app .scroll") === before) await new Promise(requestAnimationFrame);
    const shown = document.querySelector("#app .abar")?.hidden === false;
    for (let i = 0; i < 2; i++) document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2)?.click();
    await new Promise((done) => setTimeout(done, 1200));
    return { shown, top: document.querySelector("#app .scroll").scrollTop, later: document.querySelector("#app .abar")?.hidden === false, armed: Boolean(document.querySelector("#app .armed")), posts };
  })()`);
  return found.shown || found.top || found.later || found.armed || found.posts.length ? found : true;
});
await hooked("a rotation keeps the Say field, its focus and caret", "/tasks/1", SAYS, {}, async () => {
  await sleep(1000);
  const seen = [];
  await evaluate(`globalThis.saySeen = ${SAYS}; saySeen.focus()`);
  await page("Input.insertText", { text: "hello" });
  await evaluate("saySeen.setSelectionRange(2, 4)");
  for (const [width, height] of [[844, 390], [390, 844], [1440, 900], [390, 844]]) {
    await screen(width, height);
    await sleep(400);
    seen.push(await evaluate(`saySeen === ${SAYS} && document.activeElement === saySeen && saySeen.selectionStart === 2 && saySeen.selectionEnd === 4 && saySeen.getClientRects().length > 0`));
  }
  return seen.every(Boolean) || seen;
});
await screen(1440, 900);

await hooked("a task read that keeps failing polls no faster, says so in its page, and locks nothing", "/tasks/1", SAYS, {}, async () => {
  await sleep(2500);
  await evaluate('hook.fails = ["/api/tasks/1"]; polls = 0; reads.length = 0');
  change();
  await sleep(9000);
  const found = await evaluate('({ polls, reads: reads.length, stale: document.querySelector(".stale")?.innerText ?? null, conn: document.querySelector(".conn").innerText, locked: document.getElementById("act-1-cancel").getAttribute("aria-disabled"), said: [...document.querySelectorAll("#task-scroll > .note")].map((note) => note.innerText).join() })');
  return (found.polls <= 4 && found.reads <= 12 && !found.stale && found.conn === "live" && found.locked === "false" && found.said.includes("Could not read task 1 again")) ? true : found;
});
await hooked("a Cancel on a cold task page sends the revision of the task on screen", "/tasks/1", 'document.getElementById("act-1-cancel")', { cold: true, reply: refused, slow: { "/api/tasks": 8000 }, revision: { "/api/tasks/1": 4242 } }, async () => {
  for (let i = 0; i < 2; i++) {
    await evaluate('document.getElementById("act-1-cancel").click()');
    await sleep(200);
  }
  const sent = await evaluate("posts");
  return sent.length === 1 && sent[0][0] === "/api/tasks/1/cancel" && sent[0][1].revision === 4242 ? true : sent;
});
const DESK_CANCEL = 'document.querySelector("[data-act=cancel]")';
await open("/", DESK_CANCEL);
await sleep(2000);
await hooked("a Cancel on a Desk this device kept waits for the live read, then sends its revision", "/", DESK_CANCEL, { reply: refused, slow: { "/api/desk": 4000 }, revision: { "/api/desk": 4243 } }, async () => {
  const twice = async () => {
    for (let i = 0; i < 2; i++) {
      await evaluate(`${DESK_CANCEL}.click()`);
      await sleep(400);
    }
    return evaluate("posts");
  };
  const early = await twice();
  await until(`${DESK_CANCEL}?.getAttribute("aria-disabled") === "false"`, "the live read");
  const sent = await twice();
  return early.length === 0 && sent.length === 1 && sent[0][1].revision === 4243 ? true : { early, sent };
});
await hooked("a first read that failed is read again within 6 s", "/runs", 'document.querySelector("#app nav")', { cold: true, fails: ["/api/runs"] }, async () => {
  await sleep(1500);
  await evaluate("hook.fails = []");
  await sleep(6000);
  const found = await evaluate('({ conn: document.querySelector(".conn").innerText, stale: document.querySelector(".stale")?.innerText ?? null, title: document.querySelector("#app h1")?.textContent })');
  return found.conn === "live" && !found.stale && found.title === "Runs" ? true : found;
});
await hooked("a first read that fails after a move from a page that polls is read again within 8 s", "/more", 'document.querySelector("#app nav")', { cold: true, fails: ["/api/runs"] }, async () => {
  await sleep(2500);
  await evaluate(`document.querySelector('a[href="/runs"]').click()`);
  await sleep(1000);
  await evaluate("hook.fails = []");
  await sleep(8000);
  const found = await evaluate('({ conn: document.querySelector(".conn").innerText, title: document.querySelector("#app h1")?.textContent })');
  return found.conn === "live" && found.title === "Runs" ? true : found;
});
await hooked("a queue that cannot be read again says so on its page, and the app stays live", "/queue", TITLE("Queue"), {}, async () => {
  await sleep(1500);
  const rows = () => evaluate('document.querySelector(".list .scroll").innerText.length');
  const before = await rows();
  await evaluate('hook.fails = ["/api/queue"]');
  change();
  await until('document.querySelector(".stale")', "the note");
  const found = await evaluate('({ conn: document.querySelector(".conn").innerText, said: document.querySelector(".stale").innerText })');
  const kept = (await rows()) === before;
  await evaluate('hook.fails = []; document.querySelector(".stale button").click()');
  await until('!document.querySelector(".stale")', "the note to go");
  return found.conn === "live" && found.said.startsWith("Could not read the queue again: /api/queue answered 500.") && kept ? true : { ...found, kept };
});
await hooked("a page whose first read failed says why, and the app stays live", "/queue", 'document.querySelector("#app nav")', { cold: true, fails: ["/api/queue"] }, async () => {
  await until('document.getElementById("app").innerText.includes("Could not load this page: /api/queue answered 500.")', "why the page has nothing");
  return (await evaluate('document.querySelector(".conn").innerText')) === "live" || "reconnecting";
});
await hooked("an ask answers while the tasks and its task cannot be read", "/asks/1", OPTION, { reply: refused }, async () => {
  await sleep(1500);
  await evaluate('hook.fails = ["/api/tasks", "/api/tasks/1"]');
  change();
  await until('document.querySelector(".stale")', "the note");
  for (let i = 0; i < 2; i++) {
    await evaluate(`${OPTION}.click()`);
    await sleep(800);
  }
  const sent = await evaluate("posts");
  return sent.length === 1 || sent;
});
const OPEN_ASK = `document.querySelector(".list [href^='/asks/']")`;
await hooked("keys on a cold task page: S and Enter do nothing, J opens its open ask, N opens Add", "/tasks/1", SAYS, { cold: true }, async () => {
  await until(OPEN_ASK, "the task's open ask");
  for (const name of ["s", "Enter", "Escape", "1"]) await key(name);
  const still = await evaluate("location.pathname");
  const ask = await evaluate(`${OPEN_ASK}.getAttribute("href")`);
  await key("j");
  await until('location.pathname !== "/tasks/1"', "J to open the ask");
  const { errors, there } = await evaluate("({ errors, there: location.pathname })");
  await key("n");
  await until('document.getElementById("add").open', "Add by its key");
  return errors.length === 0 && still === "/tasks/1" && there === ask ? true : { errors, still, ask, there };
});
// The demo serves no tree, so the view's layout is the check's own.
await screen(1440, 260);
await hooked("a panel of a tree keeps its scroll and focus through a resize", "/asks", 'document.querySelectorAll(".tree .list").length === 2', { layout: { row: [{ panel: "asks" }, { panel: "runs" }] } }, async () => {
  await sleep(1500);
  const at = () => evaluate('({ top: document.querySelector("#app .list .scroll").scrollTop, focus: document.activeElement.getAttribute("href") })');
  await evaluate('document.querySelector("#app .list .views a").focus(); document.querySelector("#app .list .scroll").scrollTop = 60');
  const seen = [await at()];
  for (const width of [800, 1440]) {
    await screen(width, 260);
    await sleep(800);
    seen.push(await at());
  }
  return seen[0].top > 0 && seen[0].focus && seen.every((each) => JSON.stringify(each) === JSON.stringify(seen[0])) ? true : seen;
});
await screen(1440, 900);
await hooked("the Add button opens Add on a cold task page", "/tasks/1", SAYS, { cold: true }, async () => {
  await evaluate('document.querySelector("[data-action=add-open]").click()');
  await until('document.getElementById("add").open', "Add by its button");
  return true;
});
await hooked("typing to another runner's task keeps the field", "/tasks", 'document.querySelector("tr.remote")', {}, async () => {
  await sleep(1500);
  await evaluate('document.querySelector("tr.remote [data-action=remote-open]").click()');
  await until(SAYS, "the remote task's Say");
  await evaluate(`globalThis.saySeen = ${SAYS}; saySeen.focus()`);
  for (const text of ["a", "b"]) await page("Input.insertText", { text });
  await sleep(200);
  return (await evaluate(`saySeen === ${SAYS} && saySeen.value.endsWith("ab")`)) || "another field";
});
for (const [path, wanted] of [["/asks/%E0%A4%A", "No ask %E0%A4%A here"], ["/tasks/1#100%", "Say"], ["/batches/x", "No page lives at this address."]]) {
  await hooked(`${path} draws its page in the frame`, path, 'document.querySelector("#app nav")', {}, async () => {
    await until(`document.getElementById("app").innerText.includes(${JSON.stringify(wanted)})`, wanted);
    return true;
  });
}
await hooked("a link that does not decode draws its page in the frame", "/more", 'document.querySelector("#app nav")', {}, async () => {
  await evaluate('document.querySelector("#app nav a").setAttribute("href", "/asks/100%"); document.querySelector("#app nav a").click()');
  await until('document.getElementById("app").innerText.includes("No ask 100% here")', "the ask's page");
  return true;
});

await screen(390, 844);
await hooked("Back keeps the scroll of a list and of a task", "/", 'document.querySelector("#app .scroll")', {}, async () => {
  const scroll = (top) => evaluate(`document.querySelector("#app .scroll").scrollTop ${top === undefined ? "" : `= ${top}`}`);
  await sleep(1500);
  await scroll(200);
  await sleep(200);
  await evaluate(`document.querySelector('#app .scroll a[href^="/tasks/"]').click()`);
  await until(SAYS + ' || document.querySelector("#task-scroll")', "a task");
  await sleep(500);
  const opened = await scroll();
  await scroll(100);
  await sleep(200);
  await evaluate("history.back()");
  await sleep(800);
  const list = await scroll();
  await evaluate("history.forward()");
  await sleep(800);
  const task = await scroll();
  return opened === 0 && list === 200 && task > 0 ? true : { opened, list, task };
});
await screen(1440, 900);

const REVISION = 'document.querySelector(".drawer .ahead .ctx")?.innerText.match(/revision (\\d+)/)?.[1]';
await open("/asks/1", OPTION);
await sleep(2000);
await hooked("an ask drawn from what this device kept answers with the revision it drew", "/asks/1", OPTION, { reply: refused, slow: { "/api/asks": 6000 } }, async () => {
  const drawn = Number(await evaluate(REVISION));
  for (let i = 0; i < 2; i++) {
    await evaluate(`${OPTION}.click()`);
    await sleep(800);
  }
  const found = await evaluate('({ posts, landed: reads.length })');
  return found.posts.length === 1 && found.posts[0][1].revision === drawn ? true : { drawn, ...found };
});
await hooked("a refused answer's ask stays, also once the page is left and opened again", "/asks/1", OPTION, {}, async () => {
  await sleep(1500);
  await evaluate(`(async () => {
    const { asks } = await (await fetch("/api/asks", { headers: { Authorization: "Bearer demo-token" } })).json();
    hook.reply = { error: "the ask moved", ask: { ...asks.find((ask) => ask.id === 1), revision: 99 } };
    // An older read, begun before the answer, lands after its refusal; every later one fails.
    hook.late = "/api/asks";
    document.dispatchEvent(new Event("visibilitychange"));
    await new Promise((done) => setTimeout(done, 500));
    hook.fails = ["/api/asks"];
  })()`);
  for (let i = 0; i < 2; i++) {
    await evaluate(`${OPTION}.click()`);
    await sleep(800);
  }
  await until(`${REVISION} === "99"`, "the refused ask's revision");
  await sleep(5000);
  await evaluate(`document.querySelector('a[href="/runs"]').click()`);
  await until(TITLE("Runs"), "Runs");
  await evaluate("history.back()");
  await until('document.querySelector(".drawer .opt")', "the ask");
  await sleep(500);
  return (await evaluate(`${REVISION} === "99"`)) || "the older ask came back";
});
await hooked("one 5 s clock on a task page, none in a hidden tab, and one snapshot per refresh", "/tasks/1", SAYS, {}, async () => {
  await sleep(3000);
  const shown = await evaluate("clocks.size");
  await evaluate("kept = 0");
  change();
  await until("kept", "the snapshot");
  await sleep(1500);
  const kept = await evaluate("kept");
  await evaluate(TAB(true));
  const hidden = await evaluate("clocks.size");
  await evaluate(TAB(false));
  const again = await evaluate("clocks.size");
  return shown === 1 && kept === 1 && hidden === 0 && again === 1 ? true : { shown, kept, hidden, again };
});

for (const { check: own } of pages) if (own) await own({ open, holds, hooked, page, evaluate, until, sleep, change, report, screen });

socket.close();
browser.kill();
await sleep(800);
rmSync(profile, { recursive: true, force: true });
process.exit(results.every(Boolean) ? 0 : 1);
