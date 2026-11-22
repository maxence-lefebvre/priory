import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { FONTS, STRUCTURE, THEMES, palette } from "../lib/tokens.js";

// Each theme block of themes.css, as { theme: { token: [light, dark] } }.
function parse(css) {
  const themes = {};
  for (const [, name, body] of css.matchAll(/\[data-theme="([a-z]+)"\] \{([^}]*)\}/g)) {
    themes[name] = Object.fromEntries([...body.matchAll(/--([a-z0-9-]+): light-dark\(([^,]+(?:\([^)]*\))?), ([^)]+(?:\([^)]*\))?)\);/g)]
      .map(([, token, light, dark]) => [token, [light, dark]]));
  }
  return themes;
}

test("the adapter's tokens are design/tokens/themes.css, every theme and both modes", () => {
  const css = readFileSync(new URL("../../../design/tokens/themes.css", import.meta.url), "utf8");
  assert.deepEqual(THEMES, parse(css), "update adapters/herdr/lib/tokens.js from design/tokens/themes.css");
});

test("a palette resolves each light-dark pair for the mode", () => {
  assert.equal(palette("parchment", "dark").action, THEMES.parchment.action[1]);
});

test("each theme's font roles are its first family in themes.css", () => {
  const css = readFileSync(new URL("../../../design/tokens/themes.css", import.meta.url), "utf8");
  const roles = {};
  for (const [, name, body] of css.matchAll(/\[data-theme="([a-z]+)"\] \{([^}]*)\}/g)) {
    roles[name] = Object.fromEntries([...body.matchAll(/--font-([a-z]+): "([^"]+)"/g)].map(([, role, family]) => [role, family]));
  }
  assert.deepEqual(FONTS, roles, "update FONTS in adapters/herdr/lib/tokens.js from design/tokens/themes.css");
});

test("the structure tokens a pane draws with are design/tokens/structure.css", () => {
  const css = readFileSync(new URL("../../../design/tokens/structure.css", import.meta.url), "utf8");
  for (const [token, value] of Object.entries(STRUCTURE)) {
    const declared = css.match(new RegExp(`--${token}:\\s*([^;]+);`))?.[1];
    const expected = Array.isArray(value) ? `${value[0]}px/${value[1] === 1 ? "1" : `${value[1]}px`}` : `${value}px`;
    assert.equal(declared, expected, `--${token} in adapters/herdr/lib/tokens.js`);
  }
});

// The Priory bar draws Herdr's own chrome with Herdr's theme, so it alone keeps Herdr's spacing.
test("every panel spaces with the structure tokens, never a raw number", () => {
  const mods = new URL("../mods/", import.meta.url);
  const files = readdirSync(mods).filter((mod) => mod.startsWith("priory-") && mod !== "priory-tabs").flatMap((mod) => readdirSync(new URL(`${mod}/`, mods)).filter((file) => file.endsWith(".js")).map((file) => `${mod}/${file}`));
  for (const file of files) {
    const raw = readFileSync(new URL(file, mods), "utf8").match(/\.(p|px|py|pt|pb|pl|pr|gap)\(\d+\)/g);
    assert.equal(raw, null, `${file} spaces with ${raw?.join(", ")}: use STRUCTURE's space-N`);
  }
});

test("an unknown theme gives Graphite", () => {
  assert.equal(palette("nope", "light").wait, THEMES.graphite.wait[0]);
});

// The Priory bar draws Herdr's own chrome, so it alone takes Herdr's theme.
test("every panel draws Graphite unless settings.json's priory.theme names another theme", async () => {
  const kit = await import("./kit.js");
  const mods = readdirSync(new URL("../mods/", import.meta.url)).filter((mod) => mod.startsWith("priory-") && mod !== "priory-tabs");
  for (const [theme, settings] of [["graphite", null], ["graphite", { priory: {} }], ["indigo", { priory: { theme: "indigo" } }]]) {
    for (const mod of mods) {
      const { default: Panel } = await import(`../mods/${mod}/main.js`);
      kit.herdr.reset();
      if (settings) kit.herdr.files["/herdr/settings.json"] = JSON.stringify(settings);
      const panel = new Panel();
      const cx = kit.context();
      panel.init({}, cx);
      await kit.settle();
      // The first fill from the top of the tree: the panel's ground, or the usage line's bar.
      const ground = (/** @type {any} */ node) => node.calls.bg?.[0] ?? node.kids.map(kit.nodeOf).filter(Boolean).map(ground).find(Boolean);
      const fill = ground(kit.nodeOf(kit.draw(panel, cx)));
      assert.ok([THEMES[theme].bg[0], THEMES[theme].bar[0], THEMES[theme].surface[0]].includes(fill), `${mod} with ${JSON.stringify(settings)} draws ${theme}, not ${fill}`);
    }
  }
});
