// What the page reads once the revision moves, sorted.
export const reads = [
  ["/runs", "/api/desk /api/runs"],
];

// The rows are React's: a refresh, a failed read or a lost connection keeps the very elements, and
// no legacy slot draws them.
const LIST = 'document.querySelector("#app .list .scroll")';
const HELD = `({ row: ${LIST}?.querySelector("tr[data-href]"), legacy: document.querySelector("#app .legacy") })`;
export async function check({ hooked, evaluate, until, sleep, change, screen }) {
  await screen(1440, 900);
  await hooked("Runs keeps its rows, an open facet and its scroll through a refresh", "/runs", `${LIST}?.querySelector("tr[data-href]")`, {}, async () => {
    await sleep(1500);
    await evaluate(`globalThis.held = ${HELD}; document.querySelector("#app details.facet").open = true; ${LIST}.scrollTop = 20; reads.length = 0`);
    const scroll = await evaluate(`${LIST}.scrollTop`);
    change();
    await until('reads.includes("/api/runs")', "Runs to read again");
    await sleep(1000);
    const found = await evaluate(`({ same: ${HELD}.row === held.row, legacy: Boolean(${HELD}.legacy), open: document.querySelector("#app details.facet").open, scroll: ${LIST}.scrollTop })`);
    return found.same && !found.legacy && found.open && found.scroll === scroll ? true : { ...found, before: scroll };
  });
  await hooked("a row of Runs opens its task", "/runs", `${LIST}?.querySelector("tr[data-href] td:nth-child(4)")`, {}, async () => {
    const href = await evaluate(`${LIST}.querySelector("tr[data-href]").dataset.href`);
    await evaluate(`${LIST}.querySelector("tr[data-href] td:nth-child(4)").click()`);
    await until('location.pathname.startsWith("/tasks/")', "the task");
    return (await evaluate("location.pathname")) === href || href;
  });
  await hooked("Runs whose first read fails says why in the list's place, and draws its rows once it reads", "/runs", 'document.getElementById("app")?.innerText.includes("Could not load this page")', { cold: true, fails: ["/api/runs"] }, async () => {
    const said = await evaluate('document.getElementById("app").innerText.match(/Could not load this page: [^\\n]*/)?.[0] ?? ""');
    await evaluate("hook.fails = []");
    await until(`${LIST}?.querySelector("tr[data-href]")`, "the rows");
    const legacy = await evaluate('Boolean(document.querySelector("#app .legacy"))');
    return said.startsWith("Could not load this page: /api/runs answered 500.") && !legacy ? true : { said, legacy };
  });
  await hooked("offline, Runs keeps its rows and says it reconnects", "/runs", `${LIST}?.querySelector("tr[data-href]")`, {}, async () => {
    await sleep(1500);
    await evaluate(`globalThis.held = ${HELD}; hook.fails = ["/api/desk", "/api/asks", "/api/views", "/api/runs"]`);
    change();
    await until('document.querySelector(".conn").innerText === "reconnecting"', "the session to fail");
    await sleep(1000);
    const found = await evaluate(`({ same: ${HELD}.row === held.row, legacy: Boolean(${HELD}.legacy), stale: document.querySelector(".stale")?.innerText ?? "" })`);
    await evaluate("hook.fails = []");
    await until('document.querySelector(".conn").innerText === "live"', "the session to heal");
    return found.same && !found.legacy && found.stale.startsWith("Reconnecting") ? true : found;
  });
}
