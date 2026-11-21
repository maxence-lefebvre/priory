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

// How long a first tap on a command option stays armed for the second.
export const ARM_MS = 4000;

/** Whether `armed` still arms `option` of this revision of the ask, as the Herdr panel's `press` does. */
export const stillArmed = (armed, ask, option, now) =>
  armed != null && armed.id === ask.id && armed.revision === ask.revision && armed.text === option && now - armed.at < ARM_MS;
