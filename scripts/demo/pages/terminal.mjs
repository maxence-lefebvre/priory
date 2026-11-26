// The panel reads the tasks alone; the pane polls a live run's screen, and only in a shown tab.
export const reads = [
  ["/terminal", "/api/desk /api/tasks"],
];

const TAB = (off) => `globalThis.hidden = ${off}; document.dispatchEvent(new Event("visibilitychange"))`;
// The pane's Say line keeps its element, focus, text and caret, and its notes, through a refresh.
const LINE = '(() => { const line = document.getElementById("term-line"); return { same: line === globalThis.lineSeen, focused: document.activeElement === line, value: line?.value, from: line?.selectionStart, to: line?.selectionEnd, notes: [...document.querySelectorAll("[data-term] .note")].map((note) => note.textContent) }; })()';
export async function check({ open, hooked, page, evaluate, until, sleep, change, report }) {
  await open("/tasks/8", 'location.pathname === "/tasks/8" && document.querySelector("[data-term]")');
  await evaluate('reads.length = 0; document.querySelector("[data-term]").scrollIntoView()');
  const polls = () => evaluate('reads.filter((path) => path.includes("/screen")).length');
  await until('reads.some((path) => path.includes("/screen"))', "the pane to poll");
  await evaluate(TAB(true));
  await sleep(1000);
  const before = await polls();
  await sleep(4000);
  const unseen = (await polls()) - before;
  await evaluate(TAB(false));
  await sleep(3000);
  report(unseen === 0 && (await polls()) > before, "a hidden tab polls no terminal, and a shown one again", `${unseen} polls while hidden`);

  // The pane's Say line keeps its element, focus, text and caret while the task page reads again.
  await evaluate('document.querySelector("[data-action=term-type]").click()');
  await until('document.activeElement?.id === "term-line"', "the Say line");
  // React's own links on the element are too deep to send back, so the check answers true.
  await evaluate('globalThis.lineSeen = document.getElementById("term-line"); true');
  await page("Input.insertText", { text: "hello" });
  await evaluate("lineSeen.setSelectionRange(2, 4); reads.length = 0");
  change();
  await until('reads.includes("/api/tasks/8")', "the task to read again");
  await sleep(1500);
  const found = await evaluate(LINE);
  report(found.focused && found.value === "hello" && found.from === 2 && found.to === 4, "the pane's Say line keeps its focus, text and caret through a refresh", JSON.stringify(found));

  // The demo serves no pane, so the screen read is answered with one that offers the keyboard.
  const screen = { run: 108, task: 8, width: 20, height: 1, cursor: null, lines: ["$ ready"], earlier: 0, cut: 0, redacted: 0, hash: "0000000000000000", scrollback: 0, keyboard: true };
  const OPEN = 'document.getElementById("keyboard").open';
  await hooked("an open Take the keyboard sheet stays closed after Back and Forward", "/runs", 'document.querySelector("#app nav")', { answers: { "/api/runs/108/screen": screen } }, async () => {
    // Back and Forward stay in the page only after a move inside it: a link the router follows opens the task.
    await evaluate('{ const link = Object.assign(document.createElement("a"), { href: "/tasks/8" }); document.body.append(link); link.click(); link.remove(); }');
    await until('location.pathname === "/tasks/8" && document.querySelector("[data-term]")', "the task");
    await evaluate('document.querySelector("[data-term]").scrollIntoView()');
    await until('document.querySelector("[data-term] .rid") && reads.some((path) => path.startsWith("/api/runs/108/screen"))', "the screen");
    await evaluate('document.querySelector("[data-action=term-menu]").click()');
    await until('document.querySelector("[data-action=term-take-ask]")', "Take the keyboard in the menu");
    await evaluate('document.querySelector("[data-action=term-take-ask]").click()');
    await until(OPEN, "the take sheet");
    await evaluate("history.back()");
    await until('location.pathname === "/runs"', "Back");
    await evaluate("history.forward()");
    await until('location.pathname === "/tasks/8" && document.querySelector("[data-term]")', "Forward");
    await sleep(1500);
    const found = await evaluate(`({ open: ${OPEN}, drawn: Boolean(document.querySelector("#keyboard .shead")) })`);
    return !found.open && !found.drawn ? true : found;
  });
}
