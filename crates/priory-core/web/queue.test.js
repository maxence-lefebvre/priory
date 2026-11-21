import { test } from "node:test";
import assert from "node:assert/strict";
import { QUEUE_VIEWS, moveOptions, queueBands, waitReason } from "./queue.js";

// `/api/queue` as the server sends it: waiting tasks in start order, then the held ones.
const queue = {
  bands: ["blocker", "now", "today", "later"],
  waiting: [
    { id: 4, key: "ENG-231", state: "ready", band: "now", position: 1, band_position: 1, reason: null },
    { id: 8, key: "ENG-240", state: "ready", band: "later", position: 2, band_position: 1, reason: null },
    { id: 13, key: "ENG-247", state: "queued", band: "later", position: 3, band_position: 2, reason: null },
    { id: 14, key: "ENG-250", state: "queued", band: "later", position: 4, band_position: 3, reason: "waits for 2 GB of memory" },
    { id: 7, key: "ENG-239", state: "held", band: "later", position: null, band_position: null, reason: "waits for ENG-201 to merge" },
  ],
};

test("the saved views are Waiting, Held and Last day", () => {
  assert.deepEqual(QUEUE_VIEWS, ["waiting", "held", "ended"]);
});

test("the waiting tasks group by band in the factory's order, each in admission order, without held tasks or empty bands", () => {
  assert.deepEqual(queueBands(queue).map(({ band, tasks }) => [band, tasks.map((task) => task.id)]), [["now", [4]], ["later", [8, 13, 14]]]);
});

test("each waiting task says why it waits: its reason, room, a band ahead or the task ahead", () => {
  const why = (id) => waitReason(queue.waiting.find((task) => task.id === id), queueBands(queue));
  assert.equal(why(4), "Ready: starts when a runner has room.");
  assert.equal(why(8), "Ready: starts when a runner has room.");
  assert.equal(why(13), "Waits behind ENG-240 in band later.");
  assert.equal(why(14), "waits for 2 GB of memory");
  const alone = { bands: queue.bands, waiting: [{ id: 2, key: "ENG-9", state: "queued", band: "today", band_position: 1 }, { id: 3, key: "ENG-10", state: "queued", band: "later", band_position: 1 }] };
  assert.equal(waitReason(alone.waiting[1], queueBands(alone)), "Waits behind band today.");
  assert.equal(waitReason(alone.waiting[0], queueBands(alone)), "Waits for its turn: first in band today.");
});

test("the move menu offers top, up and down within the band, then every other band of the factory, each at the revision the page drew", () => {
  const bands = queueBands(queue);
  const later = (id) => bands[1].tasks.find((task) => task.id === id);
  assert.deepEqual(moveOptions(later(13), bands, queue.bands, 40), [
    { label: "To the top", body: { revision: 40, top: true } },
    { label: "Up one", body: { revision: 40, before: 8 } },
    { label: "Down one", body: { revision: 40, after: 14 } },
    { label: "To band blocker", body: { revision: 40, band: "blocker" } },
    { label: "To band now", body: { revision: 40, band: "now" } },
    { label: "To band today", body: { revision: 40, band: "today" } },
  ]);
  assert.deepEqual(moveOptions(later(8), bands, queue.bands, 40).map((option) => option.label), ["Down one", "To band blocker", "To band now", "To band today"]);
  assert.deepEqual(moveOptions(bands[0].tasks[0], bands, ["now", "later"], 7), [{ label: "To band later", body: { revision: 7, band: "later" } }]);
});
