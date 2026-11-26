// What the page reads once the revision moves, sorted.
export const reads = [
  ["/batches/1", "/api/batches/1 /api/desk /api/tasks"],
];

const GRAPH = 'document.querySelector("#app .bgraph")';

export async function check({ hooked, evaluate, until, sleep, change }) {
  await hooked("the batch page keeps its lane rows and lines through a refresh", "/batches/1", `${GRAPH}?.querySelector("svg.lanes path")`, {}, async () => {
    await sleep(1500);
    await evaluate(`globalThis.rows = [...${GRAPH}.querySelectorAll("li")]; globalThis.lines = ${GRAPH}.querySelector("svg.lanes").innerHTML; true`);
    await evaluate("reads.length = 0");
    change();
    await until('reads.includes("/api/batches/1")', "the batch to read again");
    await sleep(1000);
    const found = await evaluate(`({ same: [...${GRAPH}.querySelectorAll("li")].every((row, at) => row === rows[at]), lines: ${GRAPH}.querySelector("svg.lanes").innerHTML === lines })`);
    return found.same && found.lines ? true : found;
  });
  await hooked("a batch that cannot be read says why", "/batches/1", 'document.querySelector("#app .empty")', { cold: true, fails: ["/api/batches/1"] }, async () => {
    await until('document.querySelector("#app .empty").innerText.startsWith("Could not load batch 1")', "the batch's error");
    return true;
  });
}
