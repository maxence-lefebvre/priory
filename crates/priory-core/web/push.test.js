import { test } from "node:test";
import assert from "node:assert/strict";
import { PUSH_KINDS, confirmPush, kindBoxes, saveKinds } from "./push.js";

const DEFAULTS = { ask: true, stopped: true, intake: true, uncertain: true, cap: true, landed: false, stall: false };
const ASKS = Object.fromEntries(Object.keys(DEFAULTS).map((kind) => [kind, kind === "ask"]));
const phone = { endpoint: "https://web.push.apple.com/phone", keys: { p256dh: "p", auth: "a" } };

/** A daemon that stores the kinds it is sent over the defaults, and a device that keeps a choice. */
function world({ kept = null, refuse = null, drop = null, old = false } = {}) {
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
        for (const kind of Object.keys(body.kinds || {})) if (!(kind in DEFAULTS)) return { status: 400, body: { error: "unknown kind" } };
        stored = { ...stored, ...body.kinds };
        // The daemon stored the save, and its reply is lost once.
        if (drop && body.kinds) {
          const lost = drop;
          drop = null;
          return lost();
        }
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
  assert.deepEqual(await saveKinds(phone, ASKS, DEFAULTS, io), ASKS);
  assert.deepEqual(sent, [{ ...phone, kinds: ASKS }]);
  assert.deepEqual(device.kept, ASKS);

  const reload = await confirmPush(phone, io);
  assert.deepEqual(checked(kindBoxes(reload.kinds, false)), ["ask"]);
  assert.deepEqual(sent[1], phone);
});

test("a save of one kind sends the kept choice with it, and leaves the others unpinned", async () => {
  const { io, sent, device } = world({ kept: { cap: false } });
  await saveKinds(phone, { landed: true }, DEFAULTS, io);
  assert.deepEqual(sent[0].kinds, { cap: false, landed: true });
  assert.deepEqual(device.kept, { cap: false, landed: true });
});

test("a toggle after the daemon lost the row keeps the opt-ins and the opt-outs", async () => {
  const { io } = world({ kept: { cap: false, landed: true } });
  const kinds = await saveKinds(phone, { ask: false }, DEFAULTS, io);
  assert.deepEqual(kinds, { ...DEFAULTS, ask: false, cap: false, landed: true });
});

test("a save leaves out a kept kind the daemon does not know, and the device keeps it", async () => {
  const { io, sent, device } = world({ kept: { weather: true, cap: false } });
  await saveKinds(phone, { ask: false }, DEFAULTS, io);
  assert.deepEqual(sent[0].kinds, { cap: false, ask: false });
  assert.deepEqual(device.kept, { weather: true, cap: false, ask: false });
});

test("a kept choice the daemon will not take back is not confirmed", async () => {
  const { io } = world({ kept: ASKS, refuse: async () => ({ status: 500, body: {} }) });
  const { status, kinds, confirmed } = await confirmPush(phone, io);
  assert.deepEqual([status, kinds, confirmed], [200, DEFAULTS, false]);
});

test("a refused save puts the kept choice back", async () => {
  for (const status of [400, 401, 422]) {
    for (const kept of [null, DEFAULTS]) {
      const { io, device } = world({ kept, refuse: async () => ({ status, body: { error: "no" } }) });
      await assert.rejects(saveKinds(phone, ASKS, DEFAULTS, io), { message: "no" });
      assert.deepEqual(device.kept, kept);
    }
  }
});

test("a save whose reply is lost is unconfirmed, and the next read agrees with the device", async () => {
  const losses = [
    async () => { throw new TypeError("dropped"); },
    async () => ({ status: 502, body: {} }),
    async () => ({ status: 200, body: {} }),
  ];
  for (const lose of losses) {
    for (const committed of [true, false]) {
      let lost = false;
      const { io, device } = world({ kept: DEFAULTS, drop: committed ? lose : null });
      const { send } = io;
      if (!committed) io.send = async (body) => (body.kinds && !lost ? ((lost = true), lose()) : send(body));
      assert.equal(await saveKinds(phone, { ask: false }, DEFAULTS, io), null);
      assert.deepEqual(device.kept, { ...DEFAULTS, ask: false });

      const read = await confirmPush(phone, io);
      assert.deepEqual([read.kinds, read.confirmed], [{ ...DEFAULTS, ask: false }, true]);
      assert.deepEqual(device.kept, read.kinds);
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
