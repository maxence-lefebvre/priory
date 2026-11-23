import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import "./idb.js";
import { DOMParser, Node, parse } from "./dom.js";
import { get } from "../lib/store.js";

// Channels that do not hold the test process open once the test ends.
class Channel extends BroadcastChannel {
  constructor(name) {
    super(name);
    this.unref();
  }
}

const until = async (check, what) => {
  for (let i = 0; i < 200; i += 1) {
    if (check()) return;
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  assert.fail(`timed out waiting for ${what}`);
};

test("the side panel, signed out, shows the connect form, and a sign-in there reaches the docked panel", async () => {
  const calls = [];
  Object.assign(globalThis, {
    Node,
    DOMParser,
    BroadcastChannel: Channel,
    document: parse(readFileSync(new URL("../command.html", import.meta.url), "utf8")),
    location: new URL("chrome-extension://ekngedcpijdeijlmffkeehilmafoikkm/command.html"),
    chrome: { permissions: { request: async ({ origins }) => (calls.push(origins), true) } },
    fetch: async (url, init) => {
      calls.push([url, init.headers.Authorization]);
      return { status: 200, json: async () => ({ views: [] }) };
    },
  });
  Object.defineProperty(navigator, "locks", { configurable: true, value: { request: () => new Promise(() => {}) } });

  // The docked panel on the pull request hears sign-ins through live(), as frame.js wires it.
  const { live } = await import("../lib/live.js");
  const heard = [];
  live((message) => message.signedIn && get("config").then((config) => heard.push(config)));

  await import("../frame.js");
  const body = document.body;
  await until(() => body.querySelector("form"), "the connect form");
  assert.equal(body.querySelector(".name").textContent, "Run a command");
  assert.equal(body.querySelector(".q").textContent, "Connect to your factory");
  assert.deepEqual(body.querySelectorAll("label").map((label) => label.textContent), ["Address", "Token"]);
  assert.equal(body.querySelector("button[type=submit]").textContent, "Connect");
  assert.doesNotMatch(body.textContent, /Sign in from the panel/);

  const address = body.querySelector("#address");
  address.value = "127.0.0.1:7340";
  address.dispatch("input");
  const token = body.querySelector("#token");
  token.value = " secret ";
  token.dispatch("input");
  body.querySelector("form").dispatch("submit");

  const config = { origin: "http://127.0.0.1:7340", token: "secret" };
  await until(() => heard.length, "the docked panel to hear the sign-in");
  assert.deepEqual(heard, [config]);
  assert.deepEqual(await get("config"), config);
  assert.deepEqual(calls, [["http://127.0.0.1:7340/*"], ["http://127.0.0.1:7340/api/views", "Bearer secret"]]);
  await until(() => !document.body.querySelector("form"), "the side panel to leave the form");
  assert.match(document.body.textContent, /Tap a command option in the panel on the pull request/);
});
