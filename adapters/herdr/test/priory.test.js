import { test } from "node:test";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { autolink, follow, markdownHtml, markdownText, priory, spansHtml } from "../lib/priory.js";

const FAKE = new URL("./fake-priory", import.meta.url).pathname;

// Herdr's exec, over Node's child_process: the one IO port the library takes.
function exec(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    execFile(command, args, { timeout: options.timeout ?? 30_000 }, (error, stdout, stderr) => {
      if (error && typeof error.code !== "number") return reject(error);
      resolve({ code: error ? error.code : 0, stdout, stderr });
    });
  });
}

function fake(replies = {}) {
  const dir = mkdtempSync(join(tmpdir(), "fake-priory-"));
  process.env.FAKE_PRIORY_DIR = dir;
  process.env.FAKE_PRIORY_LOG = join(dir, "argv.log");
  writeFileSync(process.env.FAKE_PRIORY_LOG, "");
  writeFileSync(join(dir, "revision"), "1");
  for (const [verb, reply] of Object.entries(replies)) {
    writeFileSync(join(dir, `${verb}.json`), JSON.stringify(reply));
  }
  return {
    dir,
    argv: () => readFileSync(process.env.FAKE_PRIORY_LOG, "utf8").trim().split("\n").filter(Boolean).map((line) => JSON.parse(line)),
  };
}

test("reads the desk as JSON", async () => {
  const desk = { revision: 7, asks: [{ id: 3, question: "Ship it?" }] };
  const run = fake({ desk });
  const client = priory({ exec, binary: FAKE });
  assert.deepEqual(await client.desk(), desk);
  assert.deepEqual(run.argv(), [["desk", "--json"]]);
});

test("reads the desk with its snoozed items", async () => {
  const run = fake({ desk: { revision: 7, items: [] } });
  await priory({ exec, binary: FAKE }).desk({ all: true });
  assert.deepEqual(run.argv(), [["desk", "--all", "--json"]]);
});

test("a failed call rejects with priory's last error line", async () => {
  fake();
  const client = priory({ exec, binary: FAKE });
  await assert.rejects(client.desk(), { message: "priory desk: no such thing" });
});

test("waits for the next revision instead of polling", async () => {
  const run = fake();
  const client = priory({ exec, binary: FAKE });
  const waited = client.wait(1);
  setTimeout(() => writeFileSync(join(run.dir, "revision"), "2"), 100);
  assert.equal(await waited, 2);
  assert.deepEqual(run.argv(), [["wait", "--since", "1"]]);
});

test("answers with the revision read and the text as one word", async () => {
  const run = fake();
  const client = priory({ exec, binary: FAKE });
  await client.answer(12, "-x $(touch pwned); --expected 99", 4);
  assert.deepEqual(run.argv(), [["answer", "--expected", "4", "--", "12", "-x $(touch pwned); --expected 99"]]);
});

test("confirms an answer that runs the ask's command with --run before the words", async () => {
  const run = fake();
  const client = priory({ exec, binary: FAKE });
  await client.answer("review-7", "Merge", 2, { run: true });
  assert.deepEqual(run.argv(), [["answer", "--expected", "2", "--run", "--", "review-7", "Merge"]]);
});

test("reads every listing, with the factory folder where one is needed", async () => {
  const run = fake({ asks: {}, tasks: {}, task: {}, queue: {}, schedules: {}, report: {} });
  const client = priory({ exec, binary: FAKE, factory: "/srv/factory" });
  await client.asks();
  await client.tasks();
  await client.task(1);
  await client.queue();
  await client.schedules();
  await client.report();
  await client.report(5);
  assert.deepEqual(run.argv(), [
    ["asks", "--all", "--json"],
    ["tasks", "--json"],
    ["task", "1", "--json"],
    ["queue", "/srv/factory", "--json"],
    ["schedules", "/srv/factory", "--json"],
    ["report", "show", "--json"],
    ["report", "show", "5", "--json"],
  ]);
});

test("reads costs by a split, attention and flows, with the factory folder costs and flows need", async () => {
  const run = fake({ costs: {}, report: {}, flows: {} });
  const client = priory({ exec, binary: FAKE, factory: "/srv/factory" });
  await client.costs();
  await client.costs("model");
  await client.attention();
  await client.flows();
  assert.deepEqual(run.argv(), [
    ["costs", "/srv/factory", "--json"],
    ["costs", "/srv/factory", "--by", "model", "--json"],
    ["report", "attention", "--json"],
    ["flows", "/srv/factory", "--json"],
  ]);
});

test("acts through priory's verbs with argv only", async () => {
  const run = fake();
  const client = priory({ exec, binary: FAKE });
  await client.dismiss(12, 4);
  await client.snooze("ask", 12, { for: "1h" });
  await client.snooze("blocked", 3, { until: "2026-11-21T09:00:00+01:00" });
  await client.snooze("ask", 12, null);
  await client.say(7, "--step `rm -rf ~`");
  await client.pick("2026-11-20_sentry_fnd-2409", "Take", 9);
  await client.comment("fnd-2409", "--force it", 9);
  await client.reply(3, "Why?");
  await client.resolve(3);
  await client.reopen(3);
  await client.seen();
  await client.seen({ at: "2026-11-20T12:00:00Z" });
  await client.resume(4);
  await client.revive(5);
  await client.todoDone(2);
  await client.read(31);
  await client.hold(7, { reason: "waits for review" });
  await client.hold(7, { reason: "night", until: "2026-11-21T08:00:00Z" }, 11);
  await client.release(7, 12);
  await client.move(7, { band: "now", top: true });
  await client.move(7, { before: 8 }, 13);
  await client.move(7, { after: 9 });
  await client.cancel(7, 14);
  await client.pause("the disk is low");
  await client.pause();
  await client.unpause();
  assert.deepEqual(run.argv(), [
    ["dismiss", "--expected", "4", "12"],
    ["snooze", "ask", "12", "--for", "1h"],
    ["snooze", "blocked", "3", "--until", "2026-11-21T09:00:00+01:00"],
    ["snooze", "ask", "12", "--clear"],
    ["say", "7", "--step `rm -rf ~`"],
    ["report", "pick", "2026-11-20_sentry_fnd-2409", "Take", "--expected", "9"],
    ["report", "comment", "fnd-2409", "--expected", "9", "--force it"],
    ["report", "reply", "3", "Why?"],
    ["report", "resolve", "3"],
    ["report", "reopen", "3"],
    ["seen"],
    ["seen", "--at", "2026-11-20T12:00:00Z"],
    ["resume", "4"],
    ["revive", "5"],
    ["todo", "done", "2"],
    ["inbox", "read", "31"],
    ["hold", "7", "--reason", "waits for review"],
    ["hold", "7", "--reason", "night", "--until", "2026-11-21T08:00:00Z", "--expected", "11"],
    ["release", "7", "--expected", "12"],
    ["move", "7", "--band", "now", "--top"],
    ["move", "7", "--before", "8", "--expected", "13"],
    ["move", "7", "--after", "9"],
    ["cancel", "7", "--expected", "14"],
    ["pause", "the disk is low"],
    ["pause"],
    ["unpause"],
  ]);
});

test("adds a task in the factory folder, with only the chips the person changed, and returns its key", async () => {
  const run = fake();
  writeFileSync(join(run.dir, "add.json"), "cli:2026-11-21-1\n");
  const client = priory({ exec, binary: FAKE, factory: "/srv/factory" });
  assert.equal(await client.add("--band now; $(touch pwned)"), "cli:2026-11-21-1");
  await client.add("Fix the docs", { workflow: "implement", band: "now", lane: "docs/site" });
  assert.deepEqual(run.argv(), [
    ["add", "/srv/factory", "--", "--band now; $(touch pwned)"],
    ["add", "/srv/factory", "--workflow", "implement", "--band", "now", "--lane", "docs/site", "--", "Fix the docs"],
  ]);
});

test("ends a subscription and marks a task wrong or clears it, in the factory folder", async () => {
  const run = fake();
  const client = priory({ exec, binary: FAKE, factory: "/srv/factory" });
  await client.unsubscribe(7, "github:owner/repo#7");
  await client.label(7, { evidence: "https://github.com/owner/repo/pull/91", happened: "2026-10-02" });
  await client.label(7, { evidence: "https://github.com/owner/repo/pull/91" });
  await client.label(7, { clear: true });
  await priory({ exec, binary: FAKE }).label(7, { clear: true });
  assert.deepEqual(run.argv(), [
    ["unsubscribe", "7", "github:owner/repo#7"],
    ["label", "/srv/factory", "7", "wrong", "--evidence", "https://github.com/owner/repo/pull/91", "--happened", "2026-10-02"],
    ["label", "/srv/factory", "7", "wrong", "--evidence", "https://github.com/owner/repo/pull/91"],
    ["label", "/srv/factory", "7", "wrong", "--clear"],
    ["label", "7", "wrong", "--clear"],
  ]);
});

test("prints the attach command without running it", async () => {
  const run = fake();
  writeFileSync(join(run.dir, "attach.json"), "tmux -S /tmp/priory/sock attach -t 'run 4'\n");
  const client = priory({ exec, binary: FAKE });
  assert.equal(await client.attach(7), "tmux -S /tmp/priory/sock attach -t 'run 4'");
  assert.deepEqual(run.argv(), [["attach", "7", "--print"]]);
});

test("prints a read-only attach command for watching", async () => {
  const run = fake();
  writeFileSync(join(run.dir, "attach.json"), "tmux -S /tmp/priory/sock attach -r -t 'run 4'\n");
  const client = priory({ exec, binary: FAKE });
  await client.attach(7, { readOnly: true });
  assert.deepEqual(run.argv(), [["attach", "7", "--read-only", "--print"]]);
});

test("runs priory on the machine the settings name", async () => {
  const calls = [];
  const spy = async (command, args, options) => {
    calls.push([command, args, options.on]);
    return { code: 0, stdout: "{}", stderr: "" };
  };
  await priory({ exec: spy, binary: "priory", on: "desktop" }).tasks();
  assert.deepEqual(calls, [["priory", ["tasks", "--json"], "desktop"]]);
});

// Herdr loads a mod's imports only from the mod's own folder, symlinks included, so each mod
// carries a copy of the library.
// Every mod carries priory.js; a mod that draws with Priory's colors carries tokens.js too.
test("every mod carries the library unchanged", () => {
  const herdr = new URL("..", import.meta.url).pathname;
  const mods = readdirSync(join(herdr, "mods"), { withFileTypes: true }).filter((entry) => entry.isDirectory());
  for (const file of readdirSync(join(herdr, "lib"))) {
    const library = readFileSync(join(herdr, "lib", file), "utf8");
    for (const { name: mod } of mods) {
      const copy = join(herdr, "mods", mod, file);
      if (file !== "priory.js" && !existsSync(copy)) continue;
      assert.equal(readFileSync(copy, "utf8"), library, `cp adapters/herdr/lib/${file} adapters/herdr/mods/${mod}/${file}`);
    }
  }
});

test("reads the factory's links, in the folder it names", async () => {
  const links = { links: [{ pattern: "\\B#(\\d+)\\b", url: "https://github.com/{repo}/issues/$1" }] };
  const run = fake({ links });
  const client = priory({ exec, binary: FAKE, factory: "/srv/factory" });

  assert.deepEqual(await client.links(), links);
  assert.deepEqual(run.argv(), [["links", "/srv/factory", "--json"]]);
});

test("autolink returns the spans the web view links", async () => {
  const { autolink: web } = await import("../../../crates/priory-core/web/links.js");
  const { links, texts } = JSON.parse(readFileSync(new URL("./fixtures/links.json", import.meta.url), "utf8"));

  for (const [text, repo] of texts) {
    assert.deepEqual(autolink(text, links, repo), web(text, links, repo), text);
  }
  assert.deepEqual(autolink(texts[0][0], links, "acme/app"), [
    { text: "Fix " },
    { text: "ENG-7", url: "https://linear.app/acme/issue/ENG-7" },
    { text: " before " },
    { text: "#12", url: "https://github.com/acme/app/issues/12" },
    { text: " lands, then PR " },
    { text: "#3", url: "https://github.com/acme/app/issues/3" },
    { text: " and doc-4." },
  ]);
});

// Herdr's TextView drops <br>, so each line is a paragraph, and a blank line an empty one.
test("spansHtml escapes the text and the URL, keeps each line as a paragraph and links only the spans with a URL", () => {
  assert.equal(
    spansHtml([{ text: "<b>it's</b> " }, { text: "ENG-8", url: "https://x.test/?a=1&b=\"2\"" }, { text: "\nnext\n\nlast" }]),
    '<p>&lt;b&gt;it&#39;s&lt;/b&gt; <a href="https://x.test/?a=1&amp;b=&quot;2&quot;">ENG-8</a></p><p>next</p><p></p><p>last</p>',
  );
});

// Real rows of Max's report, 2026-11-22: a Markdown link whose label holds backticks.
test("markdownHtml links a Markdown link's label and keeps its backticks as written", () => {
  assert.equal(
    markdownHtml("[Error: malformed path - found `*To` without `Begin`](https://rayon.sentry.io/issues/7861662530/)", [], null),
    '<p><a href="https://rayon.sentry.io/issues/7861662530/">Error: malformed path - found `*To` without `Begin`</a></p>',
  );
});

test("markdownHtml draws bold, inline code and autolinks, and links nothing inside code", () => {
  const links = [{ pattern: String.raw`\b[A-Z]{2,5}-\d+\b`, url: "https://linear.app/i/$0" }];
  assert.equal(
    markdownHtml("**FND-2390** lands; run `ENG-7 <x>` & see ENG-8", links, null),
    '<p><strong><a href="https://linear.app/i/FND-2390">FND-2390</a></strong> lands; run <code>ENG-7 &lt;x&gt;</code> &amp; see <a href="https://linear.app/i/ENG-8">ENG-8</a></p>',
  );
});

// Herdr's TextView drops <br>, so each line of a paragraph is a paragraph of its own.
test("markdownHtml draws headings, lists and fenced code, one paragraph per line", () => {
  const text = "## Today\nfirst line\nsecond line\n\n- one\n- **two**\n\n1. a\n2) b\n\n- [x] done\n- [ ] open\n\n```sh\ncargo test\n  <b>\n```";
  assert.equal(
    markdownHtml(text, [], null),
    "<h4>Today</h4><p>first line</p><p>second line</p><ul><li>one</li><li><strong>two</strong></li></ul><ol><li>a</li><li>b</li></ol>" +
      "<ul><li>\u2611 done</li><li>\u2610 open</li></ul><pre><code>cargo test\n  &lt;b&gt;</code></pre>",
  );
});

test("markdownHtml escapes raw HTML, links only https targets and loads no image", () => {
  assert.equal(
    markdownHtml('<img src=x onerror="a()"> [bad](javascript:alert(1)) [plain](http://x.dev) ![chart](https://img.test/c.png)', [], null),
    "<p>&lt;img src=x onerror=&quot;a()&quot;&gt; [bad](javascript:alert(1)) [plain](http://x.dev) chart</p>",
  );
});

test("markdownHtml strips a heading's closing hashes and stays linear on a long heading", () => {
  assert.equal(markdownHtml("### Plan ##\n# C# #", [], null), "<h5>Plan</h5><h3>C#</h3>");
  const started = performance.now();
  markdownHtml(`# x${" ".repeat(64_000)}!`, [], null);
  assert.ok(performance.now() - started < 200, "a 64 KB heading took over 200 ms");
});

test("markdownText is the text Markdown shows, for a one-line label", () => {
  assert.equal(markdownText("[Fix `x`](https://a.test/) is **done**\n- next"), "Fix `x` is done next");
});

test("checkState, stateWords and prStack are the web view's, word for word", () => {
  const body = (/** @type {string} */ source) => source.slice(source.indexOf("/** The tile a check draws"));
  const web = readFileSync(new URL("../../../crates/priory-core/web/stack.js", import.meta.url), "utf8");
  const herdr = readFileSync(new URL("../lib/priory.js", import.meta.url), "utf8");
  assert.equal(body(herdr), body(web), "copy checkState, stateWords and prStack from adapters/herdr/lib/priory.js to crates/priory-core/web/stack.js");
});

test("routedMark, routedNote, runnerOf, seenAgo, sentNote and answeredBy are the web view's, word for word", () => {
  const helpers = (/** @type {string} */ source) => source.slice(source.indexOf('/** "with <'), source.indexOf("\n", source.indexOf("export const answeredBy")));
  const web = readFileSync(new URL("../../../crates/priory-core/web/asks.js", import.meta.url), "utf8");
  const herdr = readFileSync(new URL("../lib/priory.js", import.meta.url), "utf8");
  assert.equal(helpers(herdr), helpers(web), "copy the helpers from adapters/herdr/lib/priory.js to crates/priory-core/web/asks.js");
});

// A scripted read loop: each read and wait takes the next reply, an Error throws it, and `log` keeps the calls in order.
// It stops once the read numbered `stopAfter` has waited or backed off.
function script(reads, waits, { stopAfter }) {
  const log = [];
  let count = 0;
  const take = (replies, call) => {
    log.push(call);
    const reply = replies.shift();
    if (reply instanceof Error) return Promise.reject(reply);
    return Promise.resolve(reply);
  };
  return {
    log,
    loop: {
      read: () => { count += 1; return take(reads, ["read"]); },
      wait: (since) => take(waits, ["wait", since]),
      sleep: (ms) => { log.push(["sleep", ms]); return Promise.resolve(); },
      failed: (error) => log.push(["failed", error.message]),
      retry: 3000,
      stopped: () => count >= stopAfter && log.at(-1)[0] !== "read",
    },
  };
}

test("follow waits from the revision each read returned, and reads again after each wait", async () => {
  const { log, loop } = script([7, 9, 9], [9, 9, 12], { stopAfter: 3 });
  await follow(loop);
  assert.deepEqual(log, [["read"], ["wait", 7], ["read"], ["wait", 9], ["read"], ["wait", 9]]);
});

test("follow waits from the last wait's revision when a read has none, at first from -1", async () => {
  const { log, loop } = script([null, null, 4], [3, 4, 5], { stopAfter: 3 });
  await follow(loop);
  assert.deepEqual(log, [["read"], ["wait", -1], ["read"], ["wait", 3], ["read"], ["wait", 4]]);
});

test("follow backs off after a failed read or wait, then reads again", async () => {
  const { log, loop } = script([new Error("priory desk: no such thing"), 5, 6], [new Error("wait: daemon gone"), 7], { stopAfter: 3 });
  await follow(loop);
  assert.deepEqual(log, [
    ["read"], ["failed", "priory desk: no such thing"], ["sleep", 3000],
    ["read"], ["wait", 5], ["failed", "wait: daemon gone"], ["sleep", 3000],
    ["read"], ["wait", 6],
  ]);
});

test("follow stops before it waits, and reports no failure once stopped", async () => {
  let stop = false;
  const log = [];
  await follow({
    read: async () => { log.push("read"); stop = true; return 3; },
    wait: async () => { log.push("wait"); return 4; },
    sleep: async () => log.push("sleep"),
    failed: () => log.push("failed"),
    retry: 3000,
    stopped: () => stop,
  });
  stop = false;
  await follow({
    read: async () => { log.push("read"); stop = true; throw new Error("late"); },
    wait: async () => 0,
    sleep: async () => log.push("sleep"),
    failed: () => log.push("failed"),
    retry: 3000,
    stopped: () => stop,
  });
  assert.deepEqual(log, ["read", "read"]);
});

test("eventWords and sinceWords are the web view's, word for word", () => {
  const body = (/** @type {string} */ source) => source.slice(source.indexOf("const SOURCES"), source.indexOf("\n}\n", source.indexOf("export function sinceWords")) + 2);
  const web = readFileSync(new URL("../../../crates/priory-core/web/inbox.js", import.meta.url), "utf8");
  const herdr = readFileSync(new URL("../lib/priory.js", import.meta.url), "utf8");
  assert.equal(body(herdr), body(web), "copy SOURCES, eventWords and sinceWords from crates/priory-core/web/inbox.js to adapters/herdr/lib/priory.js");
});

test("reads the memory candidates with the factory folder, then accepts or drops one on the factory's log", async () => {
  const run = fake({ memories: { candidates: [] } });
  const client = priory({ exec, binary: FAKE, factory: "/srv/factory" });
  assert.deepEqual(await client.memories(), { candidates: [] });
  await client.acceptMemory("c-1");
  await client.dropMemory("c-2");
  assert.deepEqual(run.argv(), [
    ["memories", "/srv/factory", "--json"],
    ["memories", "accept", "/srv/factory", "c-1"],
    ["memories", "drop", "/srv/factory", "c-2"],
  ]);
});
