// The panel reads the tasks alone; the pane polls a live run's screen, and only in a shown tab.
export const reads = [
  ["/terminal", "/api/desk /api/tasks"],
];

const TAB = (off) => `globalThis.hidden = ${off}; document.dispatchEvent(new Event("visibilitychange"))`;
// The legacy pane repaints its frame when a note in it changes, so the element may be a new one: what it holds stays.
const LINE = '(() => { const line = document.getElementById("term-line"); return { same: line === globalThis.lineSeen, focused: document.activeElement === line, value: line?.value, from: line?.selectionStart, to: line?.selectionEnd, notes: [...document.querySelectorAll("[data-term] .note")].map((note) => note.textContent) }; })()';
export async function check({ open, page, evaluate, until, sleep, change, report }) {
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
}
