// What the page reads once the revision moves, sorted.
export const reads = [
  ["/reports", "/api/desk /api/reports /api/reports/latest /api/tasks"],
];

const refused = { error: "refused by the check" };
const READY = 'document.querySelector("#app .rbar")';
// The button of `scope` whose text is `text`, clicked.
const CLICK = (text, scope = "#app") => `[...document.querySelectorAll("${scope} button")].find((each) => each.textContent === ${JSON.stringify(text)}).click()`;
const ROW = (key) => `[...document.querySelectorAll("#app .rrow")].find((row) => row.querySelector(".rl1 .mono")?.textContent === "${key}")`;

export async function check({ hooked, page, evaluate, until, sleep, change, screen }) {
  await screen(1440, 900);
  const refresh = async () => {
    await evaluate("reads.length = 0");
    change();
    await until("reads.length", "a read after the move");
    await sleep(1500);
  };
  await hooked("Reports keeps the search field and a comment draft, their focus and text, through a refresh", "/reports", READY, {}, async () => {
    await sleep(1500);
    await evaluate('document.querySelector("[aria-label=\\"Search this report\\"]").click()');
    await until('document.activeElement?.id === "report-query"', "the search field's focus");
    await page("Input.insertText", { text: "cache" });
    await until('document.querySelector(".rsearch [role=status]")?.textContent === "1 found"', "one row found");
    await evaluate('globalThis.field = document.getElementById("report-query"); true');
    await refresh();
    const search = await evaluate('document.getElementById("report-query") === field && document.activeElement === field && field.value === "cache" && document.querySelectorAll("#app .rrow").length === 1');
    await evaluate('document.querySelector("[aria-label=\\"Search this report\\"]").click()');
    await evaluate(CLICK("Comment", ".rrow .racts"));
    await until('document.activeElement?.id === "report-draft"', "the comment field's focus");
    await page("Input.insertText", { text: "a draft" });
    await evaluate('globalThis.field = document.getElementById("report-draft"); true');
    await refresh();
    const draft = await evaluate('document.getElementById("report-draft") === field && document.activeElement === field && field.value === "a draft"');
    await evaluate(CLICK("Cancel"));
    return search && draft ? true : { search, draft };
  });
  await hooked("a pick waits out its Undo and Undo sends nothing; a refused pick says why under its row", "/reports", READY, { reply: refused }, async () => {
    await sleep(1500);
    await evaluate(`${ROW("ENG-250")}.querySelector(".rpick button:nth-child(2)").click()`);
    const waiting = await evaluate('document.querySelector("#app .sending")?.innerText');
    await evaluate(CLICK("Undo", ".sending"));
    await sleep(4500);
    const undone = await evaluate("posts.length === 0 && !document.querySelector('#app .sending')");
    await evaluate(`${ROW("ENG-250")}.querySelector(".rpick button:nth-child(3)").click()`);
    await sleep(5500);
    const found = await evaluate(`({ posts, note: ${ROW("ENG-250")}.querySelector(".warnline")?.innerText })`);
    const sent = JSON.stringify(found.posts) === JSON.stringify([["/api/reports/pick", { pick: "demo_agree_eng-250", option: "Drop", revision: 2 }]]);
    return waiting?.includes('Picking "Later"') && undone && sent && found.note === "Drop: Not sent: refused by the check." ? true : { waiting, undone, ...found };
  });
  await hooked("Change reopens an agreed pick and Keep closes it; Next pick reaches the open one; the arrows read the older report", "/reports", READY, {}, async () => {
    await sleep(1500);
    await evaluate(CLICK("Change"));
    const reopened = await evaluate(`${ROW("ENG-252")}.querySelectorAll(".rpick button").length === 3`);
    await evaluate(CLICK("Keep Later"));
    const kept = await evaluate(`!${ROW("ENG-252")}.querySelector(".rpick")`);
    await evaluate(CLICK("Next pick"));
    await sleep(800);
    const reached = await evaluate(`${ROW("ENG-250")}.querySelector(".rpick").contains(document.activeElement)`);
    const newest = await evaluate('document.querySelector("#app .rdate").textContent');
    await evaluate('reads.length = 0; document.querySelector("[aria-label=\\"Older report\\"]").click()');
    await until(`document.querySelector("#app .rdate").textContent !== "${newest}"`, "the older report");
    const older = await evaluate("reads.includes('/api/reports/20')");
    await evaluate('document.querySelector("[aria-label=\\"Newer report\\"]").click()');
    await until(`document.querySelector("#app .rdate").textContent === "${newest}"`, "the newest report");
    return reopened && kept && reached && older ? true : { reopened, kept, reached, older };
  });
  await hooked("a comment, a reply, a resolve and a reopen each send their verb", "/reports", READY, { reply: refused }, async () => {
    await sleep(1500);
    await evaluate(`[...${ROW("ENG-250")}.querySelectorAll("button")].find((each) => each.textContent === "Comment").click()`);
    await until('document.activeElement?.id === "report-draft"', "the comment field");
    await page("Input.insertText", { text: "hello" });
    await evaluate(CLICK("Comment", ".rcompose"));
    await until("posts.length === 1", "the comment");
    await sleep(800);
    const kept = await evaluate('document.getElementById("report-draft")?.value');
    await evaluate(CLICK("Cancel"));
    await evaluate('document.getElementById("reply-1").click()');
    await until('document.activeElement?.id === "report-draft"', "the reply field");
    await page("Input.insertText", { text: "re" });
    await evaluate(CLICK("Reply", ".rcompose"));
    await until("posts.length === 2", "the reply");
    await sleep(800);
    await evaluate(CLICK("Cancel"));
    await evaluate('document.getElementById("resolve-1").click()');
    await until("posts.length === 3", "the resolve");
    await sleep(800);
    await evaluate('document.getElementById("reopen-3").click()');
    await until("posts.length === 4", "the reopen");
    await sleep(800);
    const found = await evaluate(`({ posts, notes: [...document.querySelectorAll("#app .rrow .warnline")].map((note) => note.innerText) })`);
    const wanted = [["/api/reports/comment", { row: "eng-250", text: "hello", revision: 2 }], ["/api/reports/reply", { comment: 1, text: "re" }], ["/api/reports/resolve", { comment: 1 }], ["/api/reports/reopen", { comment: 3 }]];
    return JSON.stringify(found.posts) === JSON.stringify(wanted) && kept === "hello" && found.notes.length === 2 ? true : { kept, ...found };
  });
}
