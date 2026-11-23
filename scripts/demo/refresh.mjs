// Checks that a refresh keeps the page drawn: it changes the demo's data while a page is open, samples
// the page every animation frame, and fails on a frame that lost a section, the Say field's focus,
// value or caret, or that drew "Loading task".
// Usage: node scripts/demo/refresh.mjs DIR [CHROMIUM]
// DIR is the folder scripts/demo/serve.sh serves. CHROMIUM defaults to $CHROMIUM, then `chromium`.
// It drives Chromium over the DevTools protocol with Node's own WebSocket, so it needs no package.
import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
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
socket.addEventListener("message", ({ data }) => {
  const message = JSON.parse(data);
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
    if (await evaluate(expression)) return;
    await sleep(100);
  }
  throw new Error(`timed out waiting for ${what}`);
};
await page("Page.enable");
await page("Emulation.setDeviceMetricsOverride", { width: 1440, height: 900, deviceScaleFactor: 1, mobile: false });
await page("Page.addScriptToEvaluateOnNewDocument", { source: 'localStorage.setItem("priory.token", "demo-token")' });

// Changes a task the page does not show, so the listings change and the page draws again.
let changes = 0;
const change = () => execFileSync("sqlite3", [join(dir, "state.db"), `UPDATE tasks SET tldr = 'Probe ${++changes}' WHERE id = 2; UPDATE revision SET value = value + 1;`]);

// Samples every frame until the page drew again and a second passed, then returns what it saw.
const SAMPLE = (probe) => `new Promise((done) => {
  const app = document.getElementById("app");
  const first = app.firstElementChild;
  const frames = [];
  const start = performance.now();
  let redrawn = 0;
  const tick = () => {
    if (app.firstElementChild !== first) redrawn = redrawn || performance.now();
    frames.push((${probe})());
    if (redrawn && performance.now() - redrawn > 1000 || performance.now() - start > 30000) return done({ redrawn: Boolean(redrawn), frames });
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
})`;

const results = [];
async function check(name, path, ready, probe, bad, prepare = async () => {}) {
  await page("Page.navigate", { url: BASE + path });
  await until(ready, `${path} to draw`);
  await prepare();
  const sampled = evaluate(SAMPLE(probe));
  await sleep(300);
  change();
  const { redrawn, frames } = await sampled;
  const wrong = frames.map(bad).filter(Boolean);
  const ok = redrawn && wrong.length === 0;
  results.push(ok);
  console.log(`${ok ? "ok" : "FAILED"} ${name}: ${frames.length} frames${redrawn ? "" : ", the page never drew again"}${wrong.length ? `, ${wrong.length} wrong, first: ${wrong[0]}` : ""}`);
}

// The ask drawer keeps the task's sections it drew before the refresh.
const sections = await (async () => {
  await page("Page.navigate", { url: BASE + "/asks/1" });
  await until('document.querySelector(".drawer #trace")', "the ask drawer");
  return evaluate('["brief", "inbox", "trace"].filter((id) => document.querySelector(".drawer #" + id))');
})();
await check("the ask drawer keeps its sections", "/asks/1", 'document.querySelector(".drawer #trace")',
  `() => ${JSON.stringify(sections)}.filter((id) => !document.querySelector(".drawer #" + id))`,
  (missing) => (missing.length ? `no ${missing.join(", ")}` : null));

// The task page keeps the Say field focused, with its text and caret, and never says it is loading.
await check("the Say field keeps focus, text and caret", "/tasks/1", 'document.getElementById("say")',
  `() => { const say = document.getElementById("say"); return { focused: document.activeElement === say, value: say?.value, caret: say?.selectionStart, loading: document.getElementById("app").innerText.includes("Loading task") }; }`,
  (frame) => (!frame.focused ? "focus lost" : frame.value !== "hello" ? `text ${JSON.stringify(frame.value)}` : frame.caret !== 2 ? `caret at ${frame.caret}` : frame.loading ? "Loading task drawn" : null),
  async () => {
    await evaluate('document.getElementById("say").focus()');
    await page("Input.insertText", { text: "hello" });
    await evaluate('document.getElementById("say").setSelectionRange(2, 2)');
  });

socket.close();
browser.kill();
await sleep(200);
rmSync(profile, { recursive: true, force: true });
process.exit(results.every(Boolean) ? 0 : 1);
