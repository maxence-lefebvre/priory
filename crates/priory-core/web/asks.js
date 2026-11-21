export const ASK_VIEWS = ["open", "snoozed", "answered", "all"];

/** The groups of `askGroups` a saved view lists, in order. @param {string} view */
export const shownGroups = (view) => (view === "all" ? ["open", "snoozed", "answered", "withdrawn"] : [view]);

/** The `POST /api/snooze` body of the S key: 1 h on an open ask of a task, else null. @param {any} ask @param {number} now */
export const snoozeShortcut = (ask, now) =>
  ask?.state === "open" && ask.task && !snoozed(ask, now) ? { kind: "ask", id: ask.id, for: "1h" } : null;

export const snoozed = (ask, now) => ask.state === "open" && Boolean(ask.snoozed_until) && Date.parse(ask.snoozed_until) > now;

// The asks list's groups. An ask snoozed past `now` leaves Open for Snoozed (DESIGN.md, Snooze).
export function askGroups(asks, now) {
  const open = asks.filter((ask) => ask.state === "open");
  return {
    open: open.filter((ask) => !snoozed(ask, now)),
    snoozed: open.filter((ask) => snoozed(ask, now)),
    answered: asks.filter((ask) => ask.state === "answered").sort((a, b) => (b.answered_at || "").localeCompare(a.answered_at || "")),
    withdrawn: asks.filter((ask) => ask.state === "withdrawn").reverse(),
  };
}

/** "with <lead>" for an open ask or desk item routed to a lead, else null. @param {any} ask */
export const routedMark = (ask) => (ask?.routed_to && (ask.state ?? "open") === "open" ? `with ${ask.routed_to}` : null);

/** What an open ask's detail says of its lead, else null. @param {any} ask */
export const routedNote = (ask) => (routedMark(ask) ? `${ask.routed_to} is answering. You can still answer first: the first answer stands.` : null);

/** Who answered, as the views say it: a person, or `agent:<lead>`. Null when unrecorded. @param {string | null | undefined} by */
export const answeredBy = (by) => (!by ? null : by.startsWith("agent:") ? by : "a person");

/** Whether an ask takes free text and a dismissal: a step's or a hold's ask takes only its options. */
export const takesText = (ask) => ask?.free_text !== false;

// How long a first tap on a command option stays armed for the second.
export const ARM_MS = 4000;

/** Whether `armed` still arms `option` of this revision of the ask, as the Herdr panel's `press` does. */
export const stillArmed = (armed, ask, option, now) =>
  armed != null && armed.id === ask.id && armed.revision === ask.revision && armed.text === option && now - armed.at < ARM_MS;

/**
 * Why an ask write did not apply, for the note on the ask. @param {number} status @param {any} body
 * @param {(iso: string) => string} ago a time as the page says its age
 */
export function writeNote(status, body, ago) {
  const fresh = body?.ask;
  if (status === 409 && fresh) {
    if (fresh.state === "open") return `The question changed. Read revision ${fresh.revision} and answer again.`;
    return fresh.state === "answered" ? `Already answered ${ago(fresh.answered_at)} ago: ${fresh.answer}.` : `Withdrawn: ${fresh.withdrawn_why}.`;
  }
  if (status === 503) return "Could not send: no priory daemon runs beside this server. Start priory daemon, then send again.";
  if (status === 504 || status === 0) return "The daemon may have applied this but did not confirm it. Check the ask's state before you send again.";
  return `Could not send: ${body?.error || `the server answered ${status}`}.`;
}
