// What the page reads once the revision moves, sorted.
export const reads = [
  ["/inbox", "/api/desk /api/inbox?unrouted=1"],
];

// A Subscribe's note, the list's scroll and the button's element outlive a refresh.
const SUBSCRIBE = '[...document.querySelectorAll(".routeform button")].find((button) => button.textContent.startsWith("Subscribe"))';
export async function check({ open, evaluate, until, sleep, change, report }) {
  await open("/inbox", `document.querySelector(".ugroup") && ${SUBSCRIBE}`);
  await sleep(1500);
  await evaluate(`globalThis.subscribeSeen = ${SUBSCRIBE}; subscribeSeen.click()`);
  await until('subscribeSeen.closest(".ugroup").querySelector(".note")', "the Subscribe note");
  await evaluate('document.querySelector("#app .list.inbox .scroll").scrollTop = 30; reads.length = 0');
  const scroll = await evaluate('document.querySelector("#app .list.inbox .scroll").scrollTop');
  change();
  await until('reads.includes("/api/inbox?unrouted=1")', "the Inbox to read again");
  await sleep(1000);
  const found = await evaluate(`({ same: subscribeSeen.isConnected, note: subscribeSeen.closest(".ugroup")?.querySelector(".note")?.textContent, scroll: document.querySelector("#app .list.inbox .scroll").scrollTop })`);
  report(found.same && Boolean(found.note) && found.scroll === scroll, "the Inbox keeps a Subscribe's note, its button and its scroll through a refresh", JSON.stringify(found));
}
