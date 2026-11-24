import { test } from "node:test";
import assert from "node:assert/strict";
import { PUSH_KINDS, confirmPush, kindBoxes, saveKinds } from "./push.js";

const DEFAULTS = { ask: true, stopped: true, intake: true, uncertain: true, cap: true, landed: false, stall: false };
const ASKS = Object.fromEntries(Object.keys(DEFAULTS).map((kind) => [kind, kind === "ask"]));
const phone = { endpoint: "https://web.push.apple.com/phone", keys: { p256dh: "p", auth: "a" } };

/** A daemon that stores the kinds it is sent over the defaults, and a device that keeps a choice. */
function world({ kept = null, refuse = null, old = false } = {}) {
  const sent = [];
  const device = { kept };
  let stored = {};
  return {
    sent,
    device,
    io: {
      send: async (body) => {
        sent.push(body);
        if (refuse && body.kinds) return refuse();
        if (old) return body.kinds ? { status: 400, body: { error: "unknown field `kinds`" } } : { status: 200, body: { ok: "subscribed" } };
        stored = { ...stored, ...body.kinds };
        return { status: 200, body: { ok: "subscribed", kinds: { ...DEFAULTS, ...stored } } };
      },
      kept: async () => device.kept,
      keep: async (kinds) => { device.kept = kinds; },
    },
  };
}
const checked = (markup) => [...markup.matchAll(/data-push-kind="(\w+)" checked/g)].map(([, kind]) => kind);

test("each kind the daemon has draws a named checkbox, checked as it is stored", () => {
  const markup = kindBoxes(DEFAULTS, false);
  assert.equal(markup.match(/type="checkbox"/g).length, PUSH_KINDS.length);
  for (const [, name] of PUSH_KINDS) assert.ok(markup.includes(`<b>${name}</b>`), name);
  assert.deepEqual(checked(markup), ["ask", "stopped", "intake", "uncertain", "cap"]);
  assert.ok(!markup.includes("Nothing is chosen"));
  assert.ok(kindBoxes(DEFAULTS, true).includes('aria-disabled="true"'));
});

test("no kind chosen says so, and a daemon without kinds draws nothing", () => {
  const none = Object.fromEntries(Object.keys(DEFAULTS).map((kind) => [kind, false]));
  assert.ok(kindBoxes(none, false).includes("Nothing is chosen, so this device gets no notification."));
  assert.equal(kindBoxes(null, false), "");
});

test("a save sends the choice, keeps it, and a reload shows it", async () => {
  const { io, sent, device } = world();
  assert.deepEqual(await saveKinds(phone, ASKS, io), ASKS);
  assert.deepEqual(sent, [{ ...phone, kinds: ASKS }]);
  assert.deepEqual(device.kept, ASKS);

  const reload = await confirmPush(phone, io);
  assert.deepEqual(checked(kindBoxes(reload.kinds, false)), ["ask"]);
  assert.deepEqual(sent[1], phone);
});

test("a save of one kind leaves the others unpinned", async () => {
  const { io, sent, device } = world({ kept: { cap: false } });
  await saveKinds(phone, { landed: true }, io);
  assert.deepEqual(sent[0].kinds, { landed: true });
  assert.deepEqual(device.kept, { cap: false, landed: true });
});

test("a kept choice the daemon will not take back is not confirmed", async () => {
  const { io } = world({ kept: ASKS, refuse: async () => ({ status: 500, body: {} }) });
  const { status, kinds, confirmed } = await confirmPush(phone, io);
  assert.deepEqual([status, kinds, confirmed], [200, DEFAULTS, false]);
});

test("a refused save and a dropped connection put the kept choice back", async () => {
  for (const refuse of [async () => ({ status: 422, body: { error: "no" } }), async () => { throw new TypeError("dropped"); }]) {
    for (const kept of [null, DEFAULTS]) {
      const { io, device } = world({ kept, refuse });
      await assert.rejects(saveKinds(phone, ASKS, io));
      assert.deepEqual(device.kept, kept);
    }
  }
});

test("a daemon that lost the choice gets the kept one again, and one that has it is sent nothing more", async () => {
  const lost = world({ kept: ASKS });
  assert.deepEqual((await confirmPush(phone, lost.io)).kinds, ASKS);
  assert.deepEqual(lost.sent, [phone, { ...phone, kinds: ASKS }]);

  const same = world({ kept: DEFAULTS });
  assert.deepEqual((await confirmPush(phone, same.io)).kinds, DEFAULTS);
  assert.equal(same.sent.length, 1);
});

test("a daemon without kinds still confirms the subscription", async () => {
  const { io, sent } = world({ kept: ASKS, old: true });
  const { status, kinds } = await confirmPush(phone, io);
  assert.deepEqual([status, kinds, sent], [200, null, [phone]]);
});

test("a kept kind the daemon does not know is never sent", async () => {
  const { io, sent } = world({ kept: { ask: false, weather: true } });
  await confirmPush(phone, io);
  assert.deepEqual(sent[1].kinds, { ask: false });
});
