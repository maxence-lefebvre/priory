import { test } from "node:test";
import assert from "node:assert/strict";
import { paintRows, sayLine, screenWatch, screenRows } from "./terminal.js";

// A clock the test moves by hand, and a server whose replies the test releases one by one.
function rig() {
  const timers = [];
  const asked = [];
  let now = 0;
  const watch = screenWatch({
    read: (run, have, history) => new Promise((resolve, reject) => asked.push({ run, have, history, resolve, reject })),
    timer: (fn, ms) => { const timer = { fn, at: now + ms }; timers.push(timer); return timer; },
    clear: (timer) => { const i = timers.indexOf(timer); if (i >= 0) timers.splice(i, 1); },
    now: () => now,
  });
  const settle = () => new Promise((resolve) => setImmediate(resolve));
  return {
    watch, asked, timers,
    async tick(ms) {
      now += ms;
      for (const timer of timers.filter((timer) => timer.at <= now)) { timers.splice(timers.indexOf(timer), 1); timer.fn(); }
      await settle();
    },
    async answer(reply) { asked.at(-1).resolve(reply); await settle(); },
    async refuse(status) { asked.at(-1).reject(Object.assign(new Error(`answered ${status}`), { status })); await settle(); },
  };
}

const screen = (run, hash, lines = ["$ cargo test"]) => ({ run, task: 1, width: 200, height: 50, cursor: null, lines, redacted: 0, hash });

test("it polls a second after each reply while shown, and once at most in flight", async () => {
  const { watch, asked, tick, answer } = rig();
  watch.show(4);
  assert.deepEqual(asked.map(({ run, have }) => [run, have]), [[4, null]]);
  watch.show(4);
  await tick(5000);
  assert.equal(asked.length, 1);
  await answer(screen(4, "a"));
  assert.equal(watch.state().screen.hash, "a");
  await tick(999);
  assert.equal(asked.length, 1);
  await tick(1);
  assert.deepEqual(asked.map(({ run, have }) => [run, have]), [[4, null], [4, "a"]]);
});

test("it backs off to 3 seconds after 10 unchanged replies, and a new revision polls at once", async () => {
  const { watch, asked, tick, answer } = rig();
  watch.show(4);
  await answer(screen(4, "a"));
  for (let i = 0; i < 10; i++) {
    await tick(1000);
    await answer({ hash: "a", same: true });
  }
  await tick(2999);
  assert.equal(asked.length, 11);
  await tick(1);
  assert.equal(asked.length, 12);
  await answer(screen(4, "b"));
  await tick(1000);
  assert.equal(asked.length, 13);
  await answer({ hash: "b", same: true });
  watch.revision();
  assert.equal(asked.length, 14);
});

test("it stops while hidden and starts again when shown", async () => {
  const { watch, asked, tick, answer } = rig();
  watch.show(4);
  await answer(screen(4, "a"));
  watch.hide();
  await tick(10000);
  watch.revision();
  assert.equal(asked.length, 1);
  watch.show(4);
  assert.equal(asked.length, 2);
  assert.equal(watch.state().screen.hash, "a");
});

test("a reply for the run shown before a switch is dropped, and the switch clears the screen", async () => {
  const { watch, asked, answer } = rig();
  watch.show(4);
  await answer(screen(4, "a"));
  await new Promise((resolve) => setImmediate(resolve));
  watch.revision();
  watch.show(5);
  assert.equal(watch.state().screen, null);
  asked[1].resolve(screen(4, "late"));
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(watch.state().screen, null);
  assert.deepEqual(asked.map(({ run }) => run), [4, 4, 5]);
  await answer(screen(5, "c"));
  assert.equal(watch.state().screen.hash, "c");
});

test("a 401 and a sign-out clear the screen and stop polling", async () => {
  const { watch, asked, tick, answer, refuse } = rig();
  watch.show(4);
  await answer(screen(4, "a"));
  await tick(1000);
  await refuse(401);
  assert.equal(watch.state().screen, null);
  await tick(10000);
  assert.equal(asked.length, 2);

  watch.show(5);
  await answer(screen(5, "b"));
  watch.stop();
  assert.equal(watch.state().screen, null);
  await tick(10000);
  assert.equal(asked.length, 3);
});

test("a run that ended keeps its last screen and stops polling; a failed read keeps it and retries", async () => {
  const { watch, asked, tick, answer, refuse } = rig();
  watch.show(4);
  await answer(screen(4, "a"));
  await tick(1500);
  await refuse(503);
  assert.deepEqual([watch.state().screen.hash, watch.state().stale], ["a", true]);
  assert.equal(watch.state().at, 0);
  await tick(1000);
  await refuse(404);
  assert.deepEqual([watch.state().screen.hash, watch.state().gone], ["a", true]);
  await tick(10000);
  assert.equal(asked.length, 3);
});

test("the watch keeps no screen in local storage", async () => {
  const stored = [];
  globalThis.localStorage = { setItem: (...args) => stored.push(args), getItem: () => null, removeItem: () => {} };
  const { watch, answer } = rig();
  watch.show(4);
  await answer(screen(4, "a", ["token [redacted]"]));
  assert.deepEqual(stored, []);
  delete globalThis.localStorage;
});

test("Say sends the trimmed line to the task's say verb, clears it when said and keeps it when refused", async () => {
  const sent = [];
  const replies = [
    { status: 200, body: { ok: "task 7 will hear it" } },
    { status: 400, body: { error: "task 7 is at step review, which no agent runs" } },
    { status: 503, body: { error: "no daemon" } },
    { status: 401, body: {} },
  ];
  const post = async (path, body) => {
    sent.push([path, body]);
    const reply = replies.shift();
    if (!reply) throw new Error("the connection dropped");
    return reply;
  };
  assert.equal(await sayLine(post, 7, "   "), null);
  assert.deepEqual(sent, []);
  assert.deepEqual(await sayLine(post, 7, "  rerun clippy  "), { said: true, note: "Said: task 7 will hear it. The agent reads it once its run is idle." });
  assert.deepEqual(sent, [["/api/tasks/7/say", { text: "rerun clippy" }]]);
  assert.deepEqual(await sayLine(post, 7, "x"), { said: false, note: "Could not say: task 7 is at step review, which no agent runs." });
  assert.deepEqual(await sayLine(post, 7, "x"), { said: false, note: "Could not say: no priory daemon runs beside this server. Start priory daemon, then send again." });
  assert.deepEqual(await sayLine(post, 7, "x"), { said: false, signOut: true });
  assert.deepEqual(await sayLine(post, 7, "x"), { said: false, note: "The daemon may have queued this but did not confirm it. Check the task before you send again." });
});

test("Load earlier reads 500 more rows a tap up to 2,000, at once, and a new run starts at none", async () => {
  const { watch, asked, answer, tick } = rig();
  watch.show(4);
  assert.equal(asked[0].history, 0);
  await answer({ ...screen(4, "a"), scrollback: 3000, earlier: 0 });
  assert.equal(watch.state().more, true);
  watch.earlier();
  assert.deepEqual([asked.length, asked[1].history, asked[1].have], [2, 500, null]);
  await answer({ ...screen(4, "b"), scrollback: 3000, earlier: 500 });
  for (let i = 0; i < 4; i++) {
    watch.earlier();
    await answer({ ...screen(4, `c${i}`), scrollback: 3000, earlier: Math.min(2000, 1000 + 500 * i) });
  }
  assert.deepEqual(asked.slice(2).map(({ history }) => history), [1000, 1500, 2000, 2000]);
  assert.equal(watch.state().more, false);
  watch.show(5);
  await tick(1000);
  assert.deepEqual([asked.at(-1).run, asked.at(-1).history], [5, 0]);
  await answer({ ...screen(5, "d"), scrollback: 0, earlier: 0 });
  assert.equal(watch.state().more, false);
});

test("an unchanged screen still brings the scrollback that grew under it", async () => {
  const { watch, answer, tick } = rig();
  watch.show(4);
  await answer({ ...screen(4, "a"), scrollback: 0, earlier: 0 });
  assert.equal(watch.state().more, false);
  await tick(1000);
  await answer({ hash: "a", same: true, scrollback: 40 });
  assert.deepEqual([watch.state().screen.hash, watch.state().more], ["a", true]);
});

test("a row splits into text and hidden parts, and markup stays text", () => {
  assert.deepEqual(screenRows(["GH_TOKEN=[redacted] and <script>alert(1)</script>", ""]), [[
    { text: "GH_TOKEN=", cls: "", hidden: false },
    { text: "[redacted]", cls: "", hidden: true },
    { text: " and <script>alert(1)</script>", cls: "", hidden: false },
  ], []]);
});

test("colour codes become classes of the eight colours, carried from row to row", () => {
  const rows = screenRows([
    "\x1b[1m\x1b[31mred\x1b[39m bold \x1b[4;92;44mgreen",
    "still\x1b[0m plain \x1b[38;5;196mx\x1b[38;2;20;20;230my\x1b[38:5:6mz\x1b[7mrev\x1b[27;22;24m",
    "\x1b[1;33mT=[redacted]\x1b[0m \x1b[2Jmove\x1b[5A",
  ]);
  assert.deepEqual(rows, [
    [
      { text: "red", cls: "bo f1", hidden: false },
      { text: " bold ", cls: "bo", hidden: false },
      { text: "green", cls: "bo ul f2 b4", hidden: false },
    ],
    [
      { text: "still", cls: "bo ul f2 b4", hidden: false },
      { text: " plain ", cls: "", hidden: false },
      { text: "x", cls: "f1", hidden: false },
      { text: "y", cls: "f4", hidden: false },
      { text: "z", cls: "f6", hidden: false },
      { text: "rev", cls: "r6", hidden: false },
    ],
    [
      { text: "T=", cls: "bo f3", hidden: false },
      { text: "[redacted]", cls: "bo f3", hidden: true },
      { text: " \x1b[2Jmove\x1b[5A", cls: "", hidden: false },
    ],
  ]);
});

test("a dark RGB colour keeps its hue, and a grey falls to black or white", () => {
  const cls = (code) => screenRows([`\x1b[${code}mx`])[0][0].cls;
  assert.deepEqual(
    ["48;2;80;20;20", "48;2;20;80;30", "38;2;215;119;87", "38;2;153;153;153", "38;2;60;60;60", "38;5;52"].map(cls),
    ["b1", "b2", "f3", "f7", "f0", "f1"],
  );
});

test("reverse video fills with the text's colour, or with the default text colour", () => {
  const cls = (code) => screenRows([`\x1b[${code}mx`])[0][0].cls;
  assert.deepEqual(["31;44;7", "7", "1;44;7"].map(cls), ["r1", "rv", "bo rv"]);
});

test("any text parses: only SGR codes leave it, and classes come from the fixed set", () => {
  let seed = 7;
  const random = (n) => ((seed = (seed * 1103515245 + 12345) % 2147483648), seed % n);
  const bits = ["\x1b[", "\x1b", "[", "m", ";", ":", "38", "48", "5", "2", "255", "999", "0", "1", "7", "9", "<b>", "&", "é", "[redacted]", "\n", " "];
  const sgr = /\x1b\[[0-9;:]*m/g;
  for (let i = 0; i < 3000; i++) {
    const lines = Array.from({ length: 1 + random(3) }, () => Array.from({ length: random(16) }, () => bits[random(bits.length)]).join(""));
    const rows = screenRows(lines);
    assert.equal(rows.length, lines.length);
    const cursor = { x: random(20), y: random(lines.length) };
    assert.equal(screenRows(lines, cursor).flat().filter((part) => part.cursor).length, 1, JSON.stringify([lines, cursor]));
    rows.forEach((parts, row) => {
      assert.equal(parts.map((part) => part.text).join(""), lines[row].replace(sgr, ""), JSON.stringify(lines[row]));
      for (const part of parts) {
        assert.match(part.cls, /^((f[0-7]|b[0-7]|r[0-7]|bo|fa|it|ul|rv)( |$))*$/, JSON.stringify(lines[row]));
        assert.ok(part.text.length > 0);
        if (part.hidden) assert.equal(part.text, "[redacted]");
      }
    });
  }
});

test("a reply asked before a sign-out or a switch away and back is dropped", async () => {
  const { watch, asked, answer } = rig();
  watch.show(4);
  watch.stop();
  watch.show(4);
  asked[0].resolve(screen(4, "old"));
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(watch.state().screen, null);
  await answer(screen(4, "new"));
  assert.equal(watch.state().screen.hash, "new");
});

test("a revision during a read polls again as soon as the read ends", async () => {
  const { watch, asked, answer } = rig();
  watch.show(4);
  watch.revision();
  assert.equal(asked.length, 1);
  await answer(screen(4, "a"));
  assert.equal(asked.length, 2);
});

test("the cursor marks the cell it sits on, inside a coloured part or past the row's end", () => {
  assert.deepEqual(screenRows(["$ \x1b[32mgo\x1b[0m!", ""], { x: 3, y: 0 })[0], [
    { text: "$ ", cls: "", hidden: false },
    { text: "g", cls: "f2", hidden: false },
    { text: "o", cls: "f2", hidden: false, cursor: true },
    { text: "!", cls: "", hidden: false },
  ]);
  assert.deepEqual(screenRows(["ab", "é"], { x: 3, y: 1 })[1], [
    { text: "é", cls: "", hidden: false },
    { text: "  ", cls: "", hidden: false },
    { text: " ", cls: "", hidden: false, cursor: true },
  ]);
  assert.deepEqual(screenRows(["ab"], null), screenRows(["ab"]));
});

// A `pre` of the page: row elements whose markup the test reads, and which a repaint replaced.
function fakePre() {
  const pre = { children: [], written: [] };
  const element = () => {
    const row = { markup: "", remove: () => pre.children.splice(pre.children.indexOf(row), 1) };
    Object.defineProperty(row, "innerHTML", { set(markup) { row.markup = markup; pre.written.push(markup); } });
    return row;
  };
  pre.ownerDocument = { createElement: element };
  pre.append = (row) => pre.children.push(row);
  return pre;
}

test("a repaint writes only the rows whose markup changed, so a selection in the others stays", () => {
  const pre = fakePre();
  paintRows(pre, ["$ cargo test\n", "running 3 tests\n", "\n"]);
  const [first, second] = pre.children;
  pre.written = [];
  paintRows(pre, ["$ cargo test\n", "running 3 tests\n", "test a ... ok\n", "\n"]);
  assert.deepEqual(pre.written, ["test a ... ok\n", "\n"]);
  assert.equal(pre.children[0], first);
  assert.equal(pre.children[1], second);
  paintRows(pre, ["$ cargo test\n"]);
  assert.deepEqual(pre.children.map((row) => row.markup), ["$ cargo test\n"]);
});
