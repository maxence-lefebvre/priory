// What the page reads once the revision moves, sorted.
export const reads = [
  ["/workflow", "/api/desk"],
];

export async function check({ hooked, evaluate, until, sleep, change, screen }) {
  await screen(1440, 900);
  await hooked("Flows keeps its diagrams through a refresh and reads them no more", "/workflow", 'document.querySelector("#app .diagram")', {}, async () => {
    await sleep(1500);
    await evaluate('globalThis.diagram = document.querySelector("#app .diagram"); document.querySelector("#app details.astext").open = true; reads.length = 0');
    change();
    await until("reads.length", "a read after the move");
    await sleep(1500);
    const found = await evaluate('({ reads: reads.slice().sort().join(" "), kept: document.querySelector("#app .diagram") === diagram, open: document.querySelector("#app details.astext").open })');
    return found.reads === "/api/desk" && found.kept && found.open ? true : found;
  });
  await hooked("Flows whose first read fails say why, and Retry draws them", "/workflow", 'document.querySelector("#app [role=alert]")', { cold: true, fails: ["/api/flows"] }, async () => {
    const said = await evaluate('document.querySelector("#app [role=alert]").innerText');
    await evaluate('hook.fails.length = 0; [...document.querySelectorAll("#app button")].find((each) => each.textContent === "Retry").click()');
    await until('document.querySelector("#app .diagram")', "the diagrams");
    return said.startsWith("The flows did not load: /api/flows answered 500.") ? true : said;
  });
}
