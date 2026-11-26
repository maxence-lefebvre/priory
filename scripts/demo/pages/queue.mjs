// What the page reads once the revision moves, sorted. The Waiting view names no pull request, so it reads no tasks.
export const reads = [
  ["/queue", "/api/desk /api/queue"],
  ["/queue?view=held", "/api/desk /api/queue /api/tasks"],
];

// The rows are React's: a refresh, a failed read or a lost connection keeps the very elements and an
// open Move menu, and no legacy slot draws them.
const LIST = 'document.querySelector("#app .list .scroll")';
const MENU = 'document.querySelector("#app details.move")';
const HELD = `({ row: ${LIST}?.querySelector("tr[data-href]"), legacy: document.querySelector("#app .legacy") })`;
export async function check({ hooked, evaluate, until, sleep, change, screen, page }) {
  await screen(1440, 900);
  await hooked("the Queue keeps its rows, an open Move menu and its focus through a refresh", "/queue", MENU, {}, async () => {
    await sleep(1500);
    await evaluate(`${MENU}.querySelector("summary").click()`);
    await until(`${MENU}.open`, "the Move menu to open");
    await evaluate(`globalThis.held = ${HELD}; globalThis.menu = ${MENU}; menu.querySelector("summary").focus(); reads.length = 0`);
    change();
    await until('reads.includes("/api/queue")', "the Queue to read again");
    await sleep(1000);
    const found = await evaluate(`({ same: ${HELD}.row === held.row && ${MENU} === menu, legacy: Boolean(${HELD}.legacy), open: menu.open, focus: document.activeElement === menu.querySelector("summary") })`);
    return found.same && !found.legacy && found.open && found.focus ? true : found;
  });
  await hooked("a refused Move says why under its row, closes its menu and gives its button the focus; Esc closes a menu", "/queue", MENU, { reply: { error: "refused by the check" } }, async () => {
    await sleep(1500);
    await evaluate(`${MENU}.querySelector("summary").click()`);
    await until(`${MENU}.open`, "the Move menu to open");
    await evaluate(`${MENU}.querySelector(".pick button").click()`);
    await until('document.querySelector("#app .note.warn")', "the refusal");
    const sent = await evaluate(`({ posts, open: ${MENU}.open, focus: document.activeElement?.id, note: document.querySelector("#app .note.warn").innerText, menu: ${MENU}.querySelector("summary").id })`);
    await evaluate(`${MENU}.querySelector("summary").click()`);
    await until(`${MENU}.open`, "the Move menu to open again");
    await page("Input.dispatchKeyEvent", { type: "keyDown", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });
    await sleep(300);
    const closed = await evaluate(`!${MENU}.open && document.activeElement === ${MENU}.querySelector("summary")`);
    const moved = sent.posts.length === 1 && sent.posts[0][0].endsWith("/move") && typeof sent.posts[0][1].revision === "number";
    return moved && !sent.open && sent.focus === sent.menu && sent.note.startsWith("The state changed since this page loaded.") && closed ? true : { ...sent, closed };
  });
  await hooked("a Queue whose first read fails says why in the list's place, and draws its rows once it reads", "/queue", 'document.getElementById("app")?.innerText.includes("Could not load this page")', { cold: true, fails: ["/api/queue"] }, async () => {
    const said = await evaluate('document.getElementById("app").innerText.match(/Could not load this page: [^\\n]*/)?.[0] ?? ""');
    await evaluate("hook.fails = []");
    await until(`${LIST}?.querySelector("tr[data-href]")`, "the rows");
    const legacy = await evaluate('Boolean(document.querySelector("#app .legacy"))');
    return said.startsWith("Could not load this page: /api/queue answered 500.") && !legacy ? true : { said, legacy };
  });
  await hooked("offline, the Queue keeps its rows and says it reconnects", "/queue", `${LIST}?.querySelector("tr[data-href]")`, {}, async () => {
    await sleep(1500);
    await evaluate(`globalThis.held = ${HELD}; hook.fails = ["/api/desk", "/api/asks", "/api/views", "/api/queue"]`);
    change();
    await until('document.querySelector(".conn").innerText === "reconnecting"', "the session to fail");
    await sleep(1000);
    const found = await evaluate(`({ same: ${HELD}.row === held.row, legacy: Boolean(${HELD}.legacy), stale: document.querySelector(".stale")?.innerText ?? "" })`);
    await evaluate("hook.fails = []");
    await until('document.querySelector(".conn").innerText === "live"', "the session to heal");
    return found.same && !found.legacy && found.stale.startsWith("Reconnecting") ? true : found;
  });
}
