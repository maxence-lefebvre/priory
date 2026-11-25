// The page reads what Tasks reads; its pane polls a live run's screen, and only in a shown tab.
export const reads = [];

const TAB = (off) => `globalThis.hidden = ${off}; document.dispatchEvent(new Event("visibilitychange"))`;
export async function check({ open, evaluate, until, sleep, report }) {
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
}
