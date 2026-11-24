// The Add action's view model for the Desk and Queue panels (DESIGN.md, Add). No drawing, no IO.

/** Each value of `field` the rows carry, once, sorted. @param {any[]} rows @param {string} field @returns {string[]} */
const values = (rows, field) => [...new Set(rows.map((row) => row[field]).filter((value) => value != null))].sort();

/**
 * The dropdowns' choices: the factory's workflows and bands, and each lane a task carries. A queue
 * read without the factory folder lists no workflows, so the dropdown offers those the tasks carry.
 * @param {any} queue `priory queue --json` @param {any[]} tasks `priory tasks --json`'s tasks
 */
export function addChoices(queue, tasks) {
  const listed = [...queue.waiting, ...tasks];
  const workflow = queue.workflows?.length ? queue.workflows : values(listed, "workflow");
  return {
    workflow,
    band: queue.bands ?? [],
    lane: values(listed, "lane"),
    unpicked: { workflow: queue.default_workflow ? `default (${queue.default_workflow})` : "default", band: "default", lane: "routed" },
  };
}

/**
 * The line under the field, and whether Enter may send.
 * @param {{ text: string, sending: boolean, error: string | null, key: string | null }} add
 * @returns {{ text: string, tone: "muted" | "warn" | "ok", ready: boolean }}
 */
export function addStatus({ text, sending, error, key }) {
  const ready = !sending && text.trim() !== "";
  if (sending) return { text: "Adding…", tone: "muted", ready };
  if (error) return { text: `Add did not go: ${error}.`, tone: "warn", ready };
  if (key && !text) return { text: `Added ${key}.`, tone: "ok", ready };
  return { text: "Workflow, band and lane come from the factory's [request] table unless you pick one.", tone: "muted", ready };
}
