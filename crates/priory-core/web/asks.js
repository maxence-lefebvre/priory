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
