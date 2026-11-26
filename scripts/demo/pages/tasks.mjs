// What the page reads once the revision moves, sorted.
export const reads = [
  ["/tasks", "/api/desk /api/tasks"],
  ["/tasks/1", "/api/asks /api/desk /api/tasks /api/tasks/1"],
];

// A refresh, a failed read or a lost connection keeps the very row elements.
const LIST = 'document.querySelector("#app .list .scroll")';
const REMOTE = 'document.querySelector("#app tr.remote [data-action=remote-open]")';
const HELD = `({ row: ${LIST}?.querySelector("tr[data-href]") })`;
export async function check({ hooked, evaluate, until, sleep, change, screen }) {
  await screen(1440, 900);
  await hooked("Tasks keeps its rows, an open remote row, a facet's focus and its scroll through a refresh", "/tasks", REMOTE, {}, async () => {
    await sleep(1500);
    await evaluate(`${REMOTE}.click()`);
    await until('document.querySelector("#app tr.remote-acts")', "the remote row's actions");
    await evaluate(`globalThis.held = ${HELD}; document.querySelector("#app .facet summary").focus(); ${LIST}.scrollTop = 20; reads.length = 0`);
    const scroll = await evaluate(`${LIST}.scrollTop`);
    change();
    await until('reads.includes("/api/tasks")', "Tasks to read again");
    await sleep(1000);
    const found = await evaluate(`({ same: ${HELD}.row === held.row, open: Boolean(document.querySelector("#app tr.remote-acts")), focus: document.activeElement.matches(".facet summary"), scroll: ${LIST}.scrollTop })`);
    return found.same && found.open && found.focus && found.scroll === scroll ? true : { ...found, before: scroll };
  });
  await hooked("Tasks whose first read fails says why in the list's place, and draws its rows once it reads", "/tasks", 'document.getElementById("app")?.innerText.includes("Could not load this page")', { cold: true, fails: ["/api/tasks"] }, async () => {
    const said = await evaluate('document.getElementById("app").innerText.match(/Could not load this page: [^\\n]*/)?.[0] ?? ""');
    await evaluate("hook.fails = []");
    await until(`${LIST}?.querySelector("tr[data-href]")`, "the rows");
    return said.startsWith("Could not load this page: /api/tasks answered 500.") ? true : said;
  });
  await hooked("offline, Tasks keeps its rows and says it reconnects", "/tasks", `${LIST}?.querySelector("tr[data-href]")`, {}, async () => {
    await sleep(1500);
    await evaluate(`globalThis.held = ${HELD}; hook.fails = ["/api/desk", "/api/asks", "/api/views", "/api/tasks"]`);
    change();
    await until('document.querySelector(".conn").innerText === "reconnecting"', "the session to fail");
    await sleep(1000);
    const found = await evaluate(`({ same: ${HELD}.row === held.row, stale: document.querySelector(".stale")?.innerText ?? "" })`);
    await evaluate("hook.fails = []");
    await until('document.querySelector(".conn").innerText === "live"', "the session to heal");
    return found.same && found.stale.startsWith("Reconnecting") ? true : found;
  });
  await screen(390, 844);
  await hooked("offline on a phone, the reconnecting strip sits under the page's top bar", "/tasks", 'document.querySelector("#app .row[data-href]")', {}, async () => {
    await sleep(1500);
    await evaluate('hook.fails = ["/api/desk", "/api/asks", "/api/views", "/api/tasks"]');
    change();
    await until('document.querySelector("#app > .stale")', "the strip");
    const found = await evaluate('({ top: document.querySelector("#app .top").getBoundingClientRect().bottom, strip: document.querySelector("#app > .stale").getBoundingClientRect().top })');
    await evaluate("hook.fails = []");
    await until('document.querySelector(".conn").innerText === "live"', "the session to heal");
    return found.strip >= found.top ? true : found;
  });
  await screen(1440, 900);
}
