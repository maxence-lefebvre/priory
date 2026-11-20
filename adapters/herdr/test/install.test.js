import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readlinkSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const HERDR = new URL("..", import.meta.url).pathname.replace(/\/$/, "");
const INSTALL = join(HERDR, "install");
const MODS = readdirSync(join(HERDR, "mods"), { withFileTypes: true }).filter((entry) => entry.isDirectory()).map((entry) => entry.name);

function install(home, ...args) {
  const env = { ...process.env, HOME: home };
  delete env.HERDR_APP_HOME;
  return spawnSync(INSTALL, args, { env, encoding: "utf8" });
}

const scratch = () => mkdtempSync(join(tmpdir(), "herdr-home-"));

test("a dry run prints each link and makes none", () => {
  const home = scratch();
  const run = install(home, "--dry-run");
  assert.equal(run.status, 0, run.stderr);
  const app = join(home, ".config/herdr-app");
  assert.deepEqual(run.stdout.trim().split("\n"), [
    ...MODS.map((mod) => `link ${app}/mods/${mod} -> ${HERDR}/mods/${mod}`),
    `link ${app}/layouts/priory.json -> ${HERDR}/layouts/priory.json`,
  ]);
  assert.equal(existsSync(app), false);
});

test("links each mod and the layout, and a second run keeps them", () => {
  const home = scratch();
  assert.equal(install(home).status, 0);
  const app = join(home, ".config/herdr-app");
  for (const mod of MODS) assert.equal(readlinkSync(join(app, "mods", mod)), join(HERDR, "mods", mod));
  assert.equal(readlinkSync(join(app, "layouts/priory.json")), join(HERDR, "layouts/priory.json"));
  const again = install(home);
  assert.equal(again.status, 0, again.stderr);
  assert.match(again.stdout, /^linked already /m);
});

test("refuses to replace anything that is not its own link, and links nothing", () => {
  const home = scratch();
  const app = join(home, ".config/herdr-app");
  mkdirSync(join(app, "layouts"), { recursive: true });
  writeFileSync(join(app, "layouts/priory.json"), "{}");
  const run = install(home);
  assert.equal(run.status, 1);
  assert.match(run.stderr, /layouts\/priory\.json is not a link to this adapter; move it away first/);
  assert.equal(existsSync(join(app, "mods")), false);
});

test("refuses a Herdr folder whose layouts folder is a file, and links nothing", () => {
  const home = scratch();
  const app = join(home, ".config/herdr-app");
  mkdirSync(app, { recursive: true });
  writeFileSync(join(app, "layouts"), "");
  const run = install(home, "--dry-run");
  assert.equal(run.status, 1);
  assert.match(run.stderr, /layouts is not a folder; move it away first/);
  assert.equal(run.stdout, "");
});
