// What the page reads once the revision moves, sorted. A wide Desk opens its first ask, of task 3.
export const reads = [
  ["/", "/api/asks /api/desk /api/queue /api/tasks /api/tasks/3"],
];

const TABLE = 'document.querySelector("#app .list .tbl")';

export async function check({ hooked, evaluate, until, sleep, change, screen }) {
  const refresh = async () => {
    await evaluate("reads.length = 0");
    change();
    await until('reads.includes("/api/desk")', "the desk to read again");
    await sleep(1000);
  };
  await screen(1440, 900);
  await hooked("the Desk keeps its table, its rows and its scroll through a refresh", "/", TABLE, {}, async () => {
    await sleep(1500);
    await evaluate(`globalThis.rows = [...${TABLE}.querySelectorAll("tbody tr")]; document.querySelector("#app .list .scroll").scrollTop = 80; true`);
    await refresh();
    const found = await evaluate(`({ same: [...${TABLE}.querySelectorAll("tbody tr")].every((row, at) => row === rows[at]), scroll: document.querySelector("#app .list .scroll").scrollTop })`);
    return found.same && found.scroll === 80 ? true : found;
  });
  await screen(390, 844);
  await hooked("offline, a phone's Desk offers its asks to read and locks Done", "/", 'document.querySelector("#app .capsule")', {}, async () => {
    await sleep(1500);
    await evaluate('hook.fails = ["/api/desk", "/api/asks", "/api/views", "/api/queue", "/api/tasks"]');
    change();
    await until('document.querySelector(".conn").innerText === "reconnecting"', "the session to fail");
    await sleep(1000);
    const found = await evaluate('({ capsule: document.querySelector("#app .capsule")?.innerText, off: document.querySelector("#app .capsule")?.classList.contains("off"), done: [...document.querySelectorAll("#app [id^=todo-row-]")].every((button) => button.getAttribute("aria-disabled") === "true") })');
    await evaluate("hook.fails = []");
    await until('document.querySelector(".conn").innerText === "live"', "the session to reconnect");
    return /^Read \d+ asks, offline$/.test(found.capsule) && found.off && found.done ? true : found;
  });
  await screen(1440, 900);
}
