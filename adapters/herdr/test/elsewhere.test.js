import { test } from "node:test";
import assert from "node:assert/strict";
import { context, draw, herdr, settle, text } from "./kit.js";

const { default: Elsewhere } = await import("../mods/priory-elsewhere/main.js");

/** The placeholder for `panel`, drawn once settings.json is read. @param {string} panel @param {object} [priory] */
async function elsewhere(panel, priory) {
  herdr.reset();
  if (priory) herdr.files["/herdr/settings.json"] = JSON.stringify({ priory });
  const view = new Elsewhere();
  const cx = context();
  view.init({ panel }, cx);
  await settle();
  return text(draw(view, cx));
}

test("names the panel and links it in the web view that settings.json names", async () => {
  const shown = await elsewhere("runs", { web: "http://127.0.0.1:7070/" });

  assert.match(shown, /Runs has no Herdr panel yet\./);
  assert.match(shown, /<a href="http:\/\/127\.0\.0\.1:7070\/runs">Open Runs in the web view<\/a>/);
});

test("without a web address, says where to set one", async () => {
  const shown = await elsewhere("inbox");

  assert.match(shown, /Inbox has no Herdr panel yet\. Set "web" in settings\.json's "priory" object to link it in the web view\./);
  assert.doesNotMatch(shown, /<a /);
});
