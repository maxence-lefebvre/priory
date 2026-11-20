import { test } from "node:test";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtempSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { priory } from "../lib/priory.js";

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

test("reads every listing, with the factory folder where one is needed", async () => {
  const run = fake({ asks: {}, tasks: {}, queue: {}, schedules: {}, report: {} });
  const client = priory({ exec, binary: FAKE, factory: "/srv/factory" });
  await client.asks();
  await client.tasks();
  await client.queue();
  await client.schedules();
  await client.report();
  await client.report(5);
  assert.deepEqual(run.argv(), [
    ["asks", "--all", "--json"],
    ["tasks", "--json"],
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
  await client.hold(7, { reason: "waits for review" });
  await client.hold(7, { reason: "night", until: "2026-11-21T08:00:00Z" });
  await client.release(7);
  await client.move(7, { band: "now", top: true });
  await client.move(7, { before: 8 });
  await client.move(7, { after: 9 });
  assert.deepEqual(run.argv(), [
    ["dismiss", "--expected", "4", "12"],
    ["snooze", "ask", "12", "--for", "1h"],
    ["snooze", "blocked", "3", "--until", "2026-11-21T09:00:00+01:00"],
    ["snooze", "ask", "12", "--clear"],
    ["say", "7", "--step `rm -rf ~`"],
    ["report", "pick", "2026-11-20_sentry_fnd-2409", "Take", "--expected", "9"],
    ["seen"],
    ["hold", "7", "--reason", "waits for review"],
    ["hold", "7", "--reason", "night", "--until", "2026-11-21T08:00:00Z"],
    ["release", "7"],
    ["move", "7", "--band", "now", "--top"],
    ["move", "7", "--before", "8"],
    ["move", "7", "--after", "9"],
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
test("every mod carries the library unchanged", () => {
  const herdr = new URL("..", import.meta.url).pathname;
  const library = readFileSync(join(herdr, "lib/priory.js"), "utf8");
  const mods = readdirSync(join(herdr, "mods"), { withFileTypes: true }).filter((entry) => entry.isDirectory());
  for (const { name: mod } of mods) {
    assert.equal(readFileSync(join(herdr, "mods", mod, "priory.js"), "utf8"), library,
      `cp adapters/herdr/lib/priory.js adapters/herdr/mods/${mod}/priory.js`);
  }
});
