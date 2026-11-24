// The Add sheet's view model (DESIGN.md, Add): the body of `POST /api/request` and what its reply means.

/** The trimmed text and each chip that holds a value, or null with no text. */
export function addBody({ text, ...chips }) {
  const trimmed = text.trim();
  if (!trimmed) return null;
  return { text: trimmed, ...Object.fromEntries(Object.entries(chips).filter(([, value]) => value)) };
}

/** Why the sheet starts on another workflow than the done task's, which is not requestable. */
export function swapLine(workflow, fallback) {
  return `Workflow ${workflow} is not requestable${fallback ? `, so this starts the default, ${fallback}` : ". Pick one"}.`;
}

/** What a reply means for the draft. Status 0 is a send that got no reply: it may have arrived. */
export function addOutcome(status, body) {
  if (status === 200) return { state: "added", key: body.key, id: body.id };
  if (status === 401) return { state: "signed-out" };
  if (status === 0 || status === 504) return { state: "unsure" };
  if (status === 503) return { state: "refused", error: "no priory daemon runs beside this server. Start priory daemon, then add again" };
  return { state: "refused", error: body.error || `the server answered ${status}` };
}
