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
    await evaluate('hook.fails.length = 0; hook.slow["/api/flows"] = 1500; [...document.querySelectorAll("#app button")].find((each) => each.textContent === "Retry").click()');
    await until('document.querySelector("#app [role=status]")?.textContent === "Loading the flows…"', "Loading while Retry reads");
    await until('document.querySelector("#app .diagram")', "the diagrams");
    return said.startsWith("The flows did not load: /api/flows answered 500.") ? true : said;
  });
  await hooked("offline, Flows whose first read failed read them no more, and read them once it reconnects", "/workflow", 'document.querySelector("#app [role=alert]")', { cold: true, fails: ["/api/flows"] }, async () => {
    await sleep(1500);
    await evaluate('hook.fails = ["/api/desk", "/api/asks", "/api/views", "/api/flows"]; reads.length = 0');
    change();
    await until('document.querySelector(".conn").innerText === "reconnecting"', "the session to fail");
    await sleep(6000);
    const offline = await evaluate('({ read: reads.includes("/api/flows"), alerts: document.querySelectorAll("#app [role=alert]").length })');
    await evaluate("hook.fails = []");
    await until('document.querySelector("#app .diagram")', "the diagrams once it reconnects");
    return !offline.read && offline.alerts === 1 ? true : offline;
  });
  // A page whose draw throws says why inside the frame: the top bar and the navigation stay.
  await hooked("Flows whose draw throws keep the frame and say why", "/workflow", 'document.querySelector("#app [role=alert]")', { cold: true, answers: { "/api/flows": {} } }, async () => {
    const found = await evaluate('({ top: Boolean(document.querySelector("#app .wtop")), nav: Boolean(document.querySelector("#app nav")), title: document.querySelector("#app .list h1")?.textContent, said: document.querySelector("#app [role=alert]").innerText })');
    // The broken read stays in this device's snapshot; later checks draw from the server's.
    await evaluate('for (const key of Object.keys(localStorage)) if (key.startsWith("priory.snapshot")) localStorage.removeItem(key)');
    return found.top && found.nav && found.title === "Could not draw this page" && found.said.startsWith("The web view failed: ") ? true : found;
  });
}
