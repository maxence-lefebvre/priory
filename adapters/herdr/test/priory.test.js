import { test } from "node:test";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { autolink, priory } from "../lib/priory.js";

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

test("acts through priory's verbs with argv only", async () => {
  const run = fake();
  const client = priory({ exec, binary: FAKE });
  await client.dismiss(12, 4);
  await client.snooze("ask", 12, { for: "1h" });
  await client.snooze("blocked", 3, { until: "2026-11-21T09:00:00+01:00" });
  await client.snooze("ask", 12, null);
  await client.say(7, "--step `rm -rf ~`");
  await client.pick("2026-11-20_sentry_fnd-2409", "Take", 9);
  await client.seen();
  await client.seen({ at: "2026-11-20T12:00:00Z" });
  await client.resume(4);
  await client.revive(5);
  await client.hold(7, { reason: "waits for review" });
  await client.hold(7, { reason: "night", until: "2026-11-21T08:00:00Z" });
  await client.release(7);
  await client.move(7, { band: "now", top: true });
  await client.move(7, { before: 8 });
  await client.move(7, { after: 9 });
  await client.cancel(7);
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
    ["seen"],
    ["seen", "--at", "2026-11-20T12:00:00Z"],
    ["resume", "4"],
    ["revive", "5"],
    ["hold", "7", "--reason", "waits for review"],
    ["hold", "7", "--reason", "night", "--until", "2026-11-21T08:00:00Z"],
    ["release", "7"],
    ["move", "7", "--band", "now", "--top"],
    ["move", "7", "--before", "8"],
    ["move", "7", "--after", "9"],
    ["cancel", "7"],
    ["pause", "the disk is low"],
    ["pause"],
    ["unpause"],
  ]);
});

test("prints the attach command without running it", async () => {
  const run = fake();
  writeFileSync(join(run.dir, "attach.json"), "tmux -S /tmp/priory/sock attach -t 'run 4'\n");
  const client = priory({ exec, binary: FAKE });
  assert.equal(await client.attach(7), "tmux -S /tmp/priory/sock attach -t 'run 4'");
  assert.deepEqual(run.argv(), [["attach", "7", "--print"]]);
});

test("runs priory on the machine the settings name", async () => {
  const calls = [];
  const spy = async (command, args, options) => {
    calls.push([command, args, options.on]);
    return { code: 0, stdout: "{}", stderr: "" };
  };
  await priory({ exec: spy, binary: "priory", on: "xps13" }).tasks();
  assert.deepEqual(calls, [["priory", ["tasks", "--json"], "xps13"]]);
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
