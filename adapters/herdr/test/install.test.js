import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readlinkSync, readdirSync, symlinkSync, writeFileSync } from "node:fs";
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
  assert.deepEqual(layout.children[1].children[1].children.map((/** @type {any} */ child) => child.id), ["center", "aside-desk", "panel"]);
  assert.equal(existsSync(app), false);
});

test("links each mod and writes the layout, and a second run keeps the links and rewrites the layout", () => {
  const home = scratch();
  assert.equal(install(home, VIEWS).status, 0);
  const app = join(home, ".config/herdr-app");
  for (const mod of MODS) assert.equal(readlinkSync(join(app, "mods", mod)), join(HERDR, "mods", mod));
  const layout = JSON.parse(readFileSync(join(app, "layouts/priory.json"), "utf8"));
  assert.equal(JSON.stringify(layout).includes('"priory-elsewhere"'), false, "every panel of the default view has its mod");
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
  assert.match(JSON.parse(readFileSync(join(app, "layouts/priory.json"), "utf8")).id, /^priory-generated-/);
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

const OTHER_VIEWS = JSON.stringify({ factory: "demo", views: { default: { layout: { tabs: [{ panel: "desk" }, { panel: "costs" }] } } } });

test("records a hash of the view files in the layout's root id", () => {
  const home = scratch();
  assert.equal(install(home, VIEWS).status, 0);
  const id = JSON.parse(readFileSync(join(home, ".config/herdr-app/layouts/priory.json"), "utf8")).id;
  assert.match(id, /^priory-generated-[0-9a-f]{12}$/);
  assert.equal(install(home, VIEWS + "\n").status, 0);
  assert.equal(JSON.parse(readFileSync(join(home, ".config/herdr-app/layouts/priory.json"), "utf8")).id, id);
});

test("says whether the installed layout was built from the same view files", () => {
  const home = scratch();
  assert.doesNotMatch(install(home, VIEWS).stdout, /view files/);
  assert.match(install(home, VIEWS).stdout, /^layout is up to date with the view files$/m);
  const changed = install(home, OTHER_VIEWS);
  assert.match(changed.stdout, /^layout was older than the view files, and is rewritten$/m);
  assert.match(install(home, OTHER_VIEWS).stdout, /^layout is up to date with the view files$/m);
});

test("calls a layout from before the hash older, and a dry run says so without writing", () => {
  const home = scratch();
  const app = join(home, ".config/herdr-app");
  mkdirSync(join(app, "layouts"), { recursive: true });
  writeFileSync(join(app, "layouts/priory.json"), JSON.stringify({ id: "priory-generated", type: "row", children: [] }));
  const run = install(home, VIEWS, "--dry-run");
  assert.match(run.stdout, /^layout was older than the view files, and would be rewritten$/m);
  assert.equal(JSON.parse(readFileSync(join(app, "layouts/priory.json"), "utf8")).id, "priory-generated");
});

const MARKER = ".priory-installed";

test("--copy copies each mod with a marker holding the source commit, and links nothing", () => {
  const home = scratch();
  const run = install(home, VIEWS, "--copy");
  assert.equal(run.status, 0, run.stderr);
  const app = join(home, ".config/herdr-app");
  for (const mod of MODS) {
    assert.equal(lstatSync(join(app, "mods", mod)).isSymbolicLink(), false);
    assert.equal(readFileSync(join(app, "mods", mod, "main.js"), "utf8"), readFileSync(join(HERDR, "mods", mod, "main.js"), "utf8"));
    assert.match(readFileSync(join(app, "mods", mod, MARKER), "utf8").trim(), /^([0-9a-f]{40}|unknown)$/);
  }
  assert.match(run.stdout, new RegExp(`^copy ${app}/mods/${MODS[0]} <- ${HERDR}/mods/${MODS[0]}$`, "m"));
});

test("a later --copy replaces its own copy, files that left the source included, and nothing else", () => {
  const home = scratch();
  const app = join(home, ".config/herdr-app");
  assert.equal(install(home, VIEWS, "--copy").status, 0);
  writeFileSync(join(app, "mods", MODS[0], "stale.js"), "");
  writeFileSync(join(app, "mods", MODS[0], "main.js"), "old");
  assert.equal(install(home, VIEWS, "--copy").status, 0);
  assert.equal(existsSync(join(app, "mods", MODS[0], "stale.js")), false);
  assert.notEqual(readFileSync(join(app, "mods", MODS[0], "main.js"), "utf8"), "old");
});

test("--copy refuses a mod folder without the marker, and copies nothing", () => {
  const home = scratch();
  const app = join(home, ".config/herdr-app");
  mkdirSync(join(app, "mods", MODS[1]), { recursive: true });
  writeFileSync(join(app, "mods", MODS[1], "main.js"), "mine");
  for (const args of [["--copy"], ["--copy", "--dry-run"]]) {
    const run = install(home, VIEWS, ...args);
    assert.equal(run.status, 1);
    assert.match(run.stderr, new RegExp(`mods/${MODS[1]} is not a link or a copy this adapter made; move it away first`));
    assert.equal(run.stdout, "");
  }
  assert.equal(existsSync(join(app, "mods", MODS[0])), false);
  assert.equal(readFileSync(join(app, "mods", MODS[1], "main.js"), "utf8"), "mine");
});

test("--copy replaces a link an earlier install made, and a plain install refuses a copy", () => {
  const home = scratch();
  const app = join(home, ".config/herdr-app");
  assert.equal(install(home, VIEWS).status, 0);
  assert.equal(install(home, VIEWS, "--copy").status, 0);
  assert.equal(lstatSync(join(app, "mods", MODS[0])).isSymbolicLink(), false);
  const run = install(home, VIEWS);
  assert.equal(run.status, 1);
  assert.match(run.stderr, /is a copy: run `install --copy` again, or move it away first/);
});

test("--copy --dry-run prints each copy and changes nothing", () => {
  const home = scratch();
  const run = install(home, VIEWS, "--copy", "--dry-run");
  assert.equal(run.status, 0, run.stderr);
  assert.match(run.stdout, /^copy /m);
  assert.equal(existsSync(join(home, ".config/herdr-app")), false);
});

test("owns only the exact root ids this adapter writes", () => {
  for (const id of ["priory-generated-custom", "priory-generated-", "priory-generated-0123456789abc", 7]) {
    const home = scratch();
    const app = join(home, ".config/herdr-app");
    mkdirSync(join(app, "layouts"), { recursive: true });
    writeFileSync(join(app, "layouts/priory.json"), JSON.stringify({ id }));
    const run = install(home, VIEWS, "--dry-run");
    assert.equal(run.status, 1, String(id));
    assert.match(run.stderr, /is not a layout this adapter wrote/);
  }
});

test("--copy leaves no staging folder behind", () => {
  const home = scratch();
  assert.equal(install(home, VIEWS, "--copy").status, 0);
  assert.equal(install(home, VIEWS, "--copy").status, 0);
  assert.deepEqual(readdirSync(join(home, ".config/herdr-app/.priory-staging")), []);
});

test("removes a copy it made of a mod the adapter no longer ships, and keeps every other mod", () => {
  const home = scratch();
  const app = join(home, ".config/herdr-app");
  for (const [mod, files] of [["priory-gone", { [MARKER]: "abc\n", "main.js": "" }], ["someone-else", { "main.js": "mine" }]]) {
    mkdirSync(join(app, "mods", mod), { recursive: true });
    for (const [name, body] of Object.entries(files)) writeFileSync(join(app, "mods", mod, name), body);
  }
  const dry = install(home, VIEWS, "--copy", "--dry-run");
  assert.equal(dry.status, 0, dry.stderr);
  assert.match(dry.stdout, new RegExp(`^remove ${app}/mods/priory-gone$`, "m"));
  assert.equal(existsSync(join(app, "mods/priory-gone")), true);
  const run = install(home, VIEWS, "--copy");
  assert.equal(run.status, 0, run.stderr);
  assert.equal(existsSync(join(app, "mods/priory-gone")), false);
  assert.equal(readFileSync(join(app, "mods/someone-else/main.js"), "utf8"), "mine");
});

test("removes its own link to a mod the adapter no longer ships, and keeps a link to anything else", () => {
  const home = scratch();
  const app = join(home, ".config/herdr-app");
  mkdirSync(join(app, "mods"), { recursive: true });
  symlinkSync(join(HERDR, "mods/priory-gone"), join(app, "mods/priory-gone"));
  symlinkSync(join(home, "priory-gone"), join(app, "mods/elsewhere-gone"));
  const run = install(home, VIEWS);
  assert.equal(run.status, 0, run.stderr);
  assert.equal(lstatSync(join(app, "mods/priory-gone"), { throwIfNoEntry: false }), undefined);
  assert.equal(readlinkSync(join(app, "mods/elsewhere-gone")), join(home, "priory-gone"));
});
