// What the page reads once the revision moves, sorted.
export const reads = [
  ["/costs", "/api/attention /api/costs?by=workflow /api/desk"],
];

const PICK = (by) => `(() => { const pick = document.querySelector("#costs-split select"); pick.value = "${by}"; pick.dispatchEvent(new Event("change", { bubbles: true })); })()`;

export async function check({ hooked, evaluate, until, sleep, change, screen }) {
  await screen(1440, 900);
  await hooked("Costs reads only the split picked, and keeps its tables and the split through a refresh", "/costs", 'document.querySelector("#costs-split select")', {}, async () => {
    await sleep(1500);
    await evaluate(`reads.length = 0; ${PICK("agent")}`);
    await until('document.getElementById("costs-split")?.firstChild.textContent === "This week by agent"', "the agent split");
    const picked = await evaluate("reads.slice()");
    await evaluate('globalThis.table = document.querySelector("#costs-split table"); reads.length = 0');
    change();
    await until("reads.length", "a read after the move");
    await sleep(1500);
    const found = await evaluate('({ reads: reads.slice().sort(), kept: document.querySelector("#costs-split table") === table, split: document.querySelector("#costs-split select").value })');
    await evaluate(PICK("workflow"));
    return picked.join() === "/api/costs?by=agent" && found.kept && found.split === "agent" && found.reads.join(" ") === "/api/attention /api/costs?by=agent /api/desk" ? true : { picked, ...found };
  });
  await hooked("a failed Costs read keeps the last spend, says why, and Retry reads it again", "/costs", 'document.querySelector("#app .note.warn")', { fails: ["/api/costs?by=workflow"] }, async () => {
    const before = await evaluate('({ spend: Boolean(document.querySelector(".spend .ctbl")), alert: document.querySelector("#app .note.warn")?.innerText })');
    await evaluate('hook.fails.length = 0; reads.length = 0; [...document.querySelectorAll("#app .note.warn button")].find((each) => each.textContent === "Retry").click()');
    await until('!document.querySelector("#app .note.warn")', "the alert to go");
    const read = await evaluate("reads.slice().sort().join(' ')");
    return before.spend && before.alert.startsWith("Could not load costs: /api/costs?by=workflow answered 500.") && read === "/api/attention /api/costs?by=workflow" ? true : { ...before, read };
  });
}
