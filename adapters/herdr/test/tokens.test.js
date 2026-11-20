import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { THEMES, palette } from "../lib/tokens.js";

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

test("a palette resolves each light-dark pair for the mode, and an unknown theme gives Indigo", () => {
  assert.equal(palette("parchment", "dark").action, THEMES.parchment.action[1]);
  assert.equal(palette("nope", "light")["action-soft"], "#ECEBFC");
});
