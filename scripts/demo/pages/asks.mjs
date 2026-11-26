// What the page reads once the revision moves, sorted. An ask opened alone keeps the Desk beside it,
// and its drawer reads its task too.
export const reads = [
  ["/asks", "/api/asks /api/desk"],
  ["/asks/1", "/api/asks /api/desk /api/queue /api/tasks /api/tasks/1"],
];

const refused = { error: "refused by the check" };
const OPTION = (n) => `document.querySelectorAll(".drawer .opts .opt")[${n}]`;
const ARMED = '[...document.querySelectorAll(".drawer .opts .opt")].map((option) => option.classList.contains("armed"))';
// The button of the drawer whose text is `text`.
const BUTTON = (text) => `[...document.querySelectorAll(".drawer button")].find((each) => each.textContent === ${JSON.stringify(text)})`;

export async function check({ hooked, evaluate, until, sleep, change, page, screen }) {
  const refresh = async () => {
    await evaluate("reads.length = 0");
    change();
    await until('reads.includes("/api/asks")', "the asks to read again");
    await sleep(1000);
  };
  await screen(1440, 900);
  // Decide-first: the first press arms, a second within ARM_MS answers, a late press re-arms, a press on another option moves the arm.
  await hooked("an option arms on the first press, answers once on the second, re-arms after 4 s and moves to another option, and its element outlives a refresh", "/asks/1", OPTION(0), { reply: refused }, async () => {
    await sleep(1500);
    await evaluate(`globalThis.first = ${OPTION(0)}; first.click()`);
    const armed = await evaluate(ARMED);
    await refresh();
    const kept = await evaluate(`${OPTION(0)} === first && first.classList.contains("armed") && posts.length === 0`);
    await evaluate(`${OPTION(0)}.click()`);
    await until("posts.length", "the answer");
    await sleep(500);
    const sent = await evaluate("posts");
    await evaluate(`${OPTION(0)}.click()`);
    await sleep(4500);
    const lapsed = await evaluate(ARMED);
    await evaluate(`${OPTION(0)}.click()`);
    await sleep(300);
    await evaluate(`${OPTION(1)}.click()`);
    await sleep(300);
    const moved = await evaluate(ARMED);
    const note = await evaluate('document.querySelector(".drawer .warnline")?.innerText');
    const ok = JSON.stringify(armed) === "[true,false]" && kept && sent.length === 1 && sent[0][0] === "/api/asks/1/answer" && sent[0][1].option === "open" && sent[0][1].revision === 1
      && JSON.stringify(lapsed) === "[false,false]" && JSON.stringify(moved) === "[false,true]" && (await evaluate("posts.length")) === 1 && note === "Could not send: refused by the check.";
    return ok || { armed, kept, sent, lapsed, moved, note };
  });
  await hooked("a written draft keeps its field through a refresh, a rotation and a Next ask and back", "/asks/1", BUTTON("Write an answer"), {}, async () => {
    await sleep(1500);
    await evaluate(`${BUTTON("Write an answer")}.click()`);
    await until('document.activeElement?.id === "draft"', "the draft's focus");
    await page("Input.insertText", { text: "a draft" });
    await evaluate('globalThis.field = document.getElementById("draft"); true');
    await refresh();
    const refreshed = await evaluate('document.getElementById("draft") === field && document.activeElement === field && field.value === "a draft"');
    await evaluate("field.setSelectionRange(2, 5)");
    await screen(390, 844);
    await sleep(800);
    const rotated = await evaluate('(() => { const draft = document.getElementById("draft"); return draft === document.activeElement && draft.selectionStart === 2 && draft.selectionEnd === 5 ? draft.value : { value: draft?.value, focused: draft === document.activeElement, from: draft?.selectionStart }; })()');
    await screen(1440, 900);
    await sleep(800);
    await evaluate(`document.querySelector('.drawer [aria-label="Next ask"]').click()`);
    await until('location.pathname !== "/asks/1"', "the next ask");
    await sleep(500);
    await evaluate("history.back()");
    await until('document.getElementById("draft")', "the draft again");
    const back = await evaluate('document.getElementById("draft").value');
    return refreshed && rotated === "a draft" && back === "a draft" ? true : { refreshed, rotated, back };
  });
  await hooked("the Open view reads no older asks once the Answered view loaded them", "/asks?view=answered", 'document.querySelector("#app .list [data-older]")', {}, async () => {
    await sleep(1500);
    await evaluate('document.querySelector("#app .list [data-older]").scrollIntoView()');
    await until('reads.includes("/api/asks?all=1")', "the older asks");
    await sleep(1000);
    await evaluate('document.querySelector(\'#app .views a[href^="/asks?view=open"]\').click()');
    await sleep(1000);
    await refresh();
    const read = await evaluate("reads.filter((path) => path.startsWith(\"/api/asks\"))");
    return read.length && read.every((path) => path === "/api/asks") ? true : read;
  });
  await hooked("Snooze and Dismiss each send once for two quick taps", "/asks/3", BUTTON("Dismiss"), { reply: refused }, async () => {
    await sleep(1500);
    await evaluate(`(() => { const button = ${BUTTON("Snooze 1 h")}; button.click(); button.click(); })()`);
    await sleep(800);
    await evaluate(`(() => { const button = ${BUTTON("Dismiss")}; button.click(); button.click(); })()`);
    await sleep(800);
    const sent = await evaluate("posts.map(([path]) => path)");
    return JSON.stringify(sent) === JSON.stringify(["/api/snooze", "/api/asks/3/dismiss"]) ? true : sent;
  });
  await hooked("offline, an ask says Reconnect to answer, sends nothing, and reads no task", "/asks/1", OPTION(0), {}, async () => {
    await sleep(1500);
    await evaluate('hook.fails = ["/api/desk", "/api/asks", "/api/views", "/api/tasks", "/api/tasks/1"]; reads.length = 0');
    change();
    await until('document.querySelector(".conn").innerText === "reconnecting"', "the session to fail");
    await sleep(3000);
    await evaluate(`${OPTION(0)}.click(); ${OPTION(0)}.click()`);
    await sleep(500);
    const found = await evaluate(`({ note: document.getElementById("answer-locked")?.innerText, off: ${OPTION(0)}.getAttribute("aria-disabled"), posts: posts.length, task: reads.filter((path) => path === "/api/tasks/1").length, unread: document.querySelector(".drawer .scroll > .note")?.innerText ?? null })`);
    await evaluate("hook.fails = []");
    await until('document.querySelector(".conn").innerText === "live"', "the session to reconnect");
    return found.note === "Reconnect to answer." && found.off === "true" && found.posts === 0 && found.task <= 1 && found.unread === null ? true : found;
  });
  await screen(390, 844);
  await hooked("the answer bar keeps its element and stays shown through a refresh, and its options open the sheet", "/asks/1", 'document.querySelector("#app .opts")', {}, async () => {
    await sleep(1500);
    await evaluate('document.querySelector("#app .scroll").scrollTop = 600');
    await until('document.querySelector("#app .abar")?.hidden === false', "the answer bar");
    await evaluate('globalThis.bar = document.querySelector("#app .abar"); true');
    await refresh();
    const kept = await evaluate('document.querySelector("#app .abar") === bar && !bar.hidden');
    await evaluate('bar.querySelector(".btn.secondary").click()');
    await until('document.getElementById("answer").open', "the answer sheet");
    const sheet = await evaluate('document.querySelectorAll("#answer .opt").length');
    await evaluate('document.querySelector("#answer [aria-label=Close]").click()');
    return kept && sheet === 2 ? true : { kept, sheet };
  });
  await screen(1440, 900);
}
