// Whether a task can hear `say`, by the daemon's own rule (tools.rs, `say`): an active task at a
// step a model decides, with a run. The panel says where the text goes before the tap.

const ENDED = new Set(["done", "failed", "dropped"]);

const decider = (flows, task) =>
  flows?.workflows?.find((workflow) => workflow.name === task.workflow)?.steps?.find((step) => step.id === task.step)?.decides;

/** Whether say reaches `task`, and the note under the buttons. */
export function sayRoute(task, flows) {
  const name = `Task ${task.id}`;
  if (ENDED.has(task.state)) return { enabled: false, note: `${name} is ${task.state}.` };
  const decides = decider(flows, task);
  if (decides !== "model") return { enabled: false, note: `${name} waits at ${task.step}, a ${decides ?? "human"} step. Say reaches it once it moves.` };
  if (!task.runs?.length) return { enabled: false, note: `${name} has no run yet. Say reaches it once its agent started.` };
  const open = (task.asks ?? []).filter((ask) => ask.state === "open").map((ask) => `Ask ${ask.id}`);
  const wakes = task.state === "asleep" ? "It wakes, works" : "It works";
  const stays = open.length ? ` ${open.join(", ")} ${open.length > 1 ? "stay" : "stays"} open.` : "";
  return { enabled: true, note: `Sent with say to task ${task.id}'s agent. ${wakes} in its worktree and pushes.${stays}` };
}

/** The fixed text Update branch or Resolve conflicts says: a merge of the base, so no force push. */
export function fixText(kind, branch, base) {
  const update = `Bring ${branch} up to date with ${base} by merging it, resolve any conflicts, run the tests and push.`;
  return kind === "conflicts" ? `GitHub reports conflicts between ${branch} and ${base}. ${update}` : update;
}
