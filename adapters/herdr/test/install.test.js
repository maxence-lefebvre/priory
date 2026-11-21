import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readlinkSync, readdirSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const HERDR = new URL("..", import.meta.url).pathname.replace(/\/$/, "");
const INSTALL = join(HERDR, "install");
const MODS = readdirSync(join(HERDR, "mods"), { withFileTypes: true }).filter((entry) => entry.isDirectory()).map((entry) => entry.name);
const VIEWS = JSON.stringify({ factory: "demo", views: { default: { layout: { tabs: [{ panel: "desk" }, { panel: "runs" }] } } } });

/** Runs install with the views JSON on its standard input, as `priory views --json | install` does. */
function install(home, input, ...args) {
  const env = { ...process.env, HOME: home };
  delete env.HERDR_APP_HOME;
  return spawnSync(INSTALL, args, { env, encoding: "utf8", input });
}

const scratch = () => mkdtempSync(join(tmpdir(), "herdr-home-"));

test("a dry run prints each link and the layout it would write, and makes none", () => {
  const home = scratch();
  const run = install(home, VIEWS, "--dry-run");
  assert.equal(run.status, 0, run.stderr);
  const app = join(home, ".config/herdr-app");
  const [links, written] = run.stdout.split(/^write .*\n/m);
  assert.deepEqual(links.trim().split("\n"), MODS.map((mod) => `link ${app}/mods/${mod} -> ${HERDR}/mods/${mod}`));
  assert.match(run.stdout, new RegExp(`^write ${app}/layouts/priory.json$`, "m"));
  const layout = JSON.parse(written);
  assert.deepEqual(layout.children[1].children[1].children.map((/** @type {any} */ child) => child.id), ["main", "view"]);
  assert.equal(existsSync(app), false);
});

test("links each mod and writes the layout, and a second run keeps the links and rewrites the layout", () => {
  const home = scratch();
  assert.equal(install(home, VIEWS).status, 0);
  const app = join(home, ".config/herdr-app");
  for (const mod of MODS) assert.equal(readlinkSync(join(app, "mods", mod)), join(HERDR, "mods", mod));
  const layout = JSON.parse(readFileSync(join(app, "layouts/priory.json"), "utf8"));
  assert.equal(JSON.stringify(layout).includes('"priory-elsewhere"'), true);
  const again = install(home, VIEWS);
  assert.equal(again.status, 0, again.stderr);
  assert.match(again.stdout, /^linked already /m);
  assert.match(again.stdout, /^write .*layouts\/priory\.json$/m);
});

test("replaces the layout link an earlier install made", () => {
  const home = scratch();
  const app = join(home, ".config/herdr-app");
  mkdirSync(join(app, "layouts"), { recursive: true });
  symlinkSync(join(HERDR, "layouts/priory.json"), join(app, "layouts/priory.json"));
  const run = install(home, VIEWS);
  assert.equal(run.status, 0, run.stderr);
  assert.equal(JSON.parse(readFileSync(join(app, "layouts/priory.json"), "utf8")).id, "priory-generated");
});

test("refuses a priory.json it did not write, and links nothing", () => {
  const home = scratch();
  const app = join(home, ".config/herdr-app");
  mkdirSync(join(app, "layouts"), { recursive: true });
  writeFileSync(join(app, "layouts/priory.json"), "{}");
  for (const args of [[], ["--dry-run"]]) {
    const run = install(home, VIEWS, ...args);
    assert.equal(run.status, 1);
    assert.match(run.stderr, /layouts\/priory\.json is not a layout this adapter wrote; move it away first/);
    assert.equal(run.stdout, "");
  }
  assert.equal(existsSync(join(app, "mods")), false);
});

test("owns a priory.json by its root id, whatever its formatting", () => {
  const home = scratch();
  const app = join(home, ".config/herdr-app");
  mkdirSync(join(app, "layouts"), { recursive: true });
  writeFileSync(join(app, "layouts/priory.json"), JSON.stringify({ id: "priory-generated", type: "row", children: [] }));
  assert.equal(install(home, VIEWS, "--dry-run").status, 0);
  writeFileSync(join(app, "layouts/priory.json"), '{\n  "id": "mine",\n  "children": [\n  "id": "priory-generated",\n  ]\n}\n');
  const run = install(home, VIEWS, "--dry-run");
  assert.equal(run.status, 1);
  assert.match(run.stderr, /is not a layout this adapter wrote/);
});

test("refuses input that is not the views JSON, and links nothing", () => {
  const home = scratch();
  const run = install(home, "", "--dry-run");
  assert.equal(run.status, 1);
  assert.match(run.stderr, /pipe `priory views --json` into it/);
  assert.equal(run.stdout, "");
});

test("refuses a Herdr folder whose layouts folder is a file, and links nothing", () => {
  const home = scratch();
  const app = join(home, ".config/herdr-app");
  mkdirSync(app, { recursive: true });
  writeFileSync(join(app, "layouts"), "");
  const run = install(home, VIEWS, "--dry-run");
  assert.equal(run.status, 1);
  assert.match(run.stderr, /layouts is not a folder; move it away first/);
  assert.equal(run.stdout, "");
});
