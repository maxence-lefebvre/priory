// The view model of the web view's Costs panel: `/api/costs` and `/api/attention` in, stat tiles,
// bars and table rows out. No drawing, no IO.

const usd = new Intl.NumberFormat("en", { style: "currency", currency: "USD" });
export const money = (/** @type {number | null} */ value) => (value == null ? "-" : usd.format(value));

/** The change from `before` to `now`, or null when either is missing or `before` is 0. @param {number | null} now @param {number | null} before */
function delta(now, before) {
  if (now == null || !before) return null;
  const percent = Math.round(((now - before) / before) * 100);
  return { text: `${percent > 0 ? "+" : ""}${percent}% vs last week`, direction: percent > 0 ? "up" : percent < 0 ? "down" : "flat" };
}

const perMerge = (/** @type {any} */ window) => (window.cost_usd == null || !window.merges ? null : window.cost_usd / window.merges);

/** This window against the one before it: cost, merges and cost per merge. @param {any} costs */
export function costTiles(costs) {
  const { current, previous } = costs;
  return [
    { label: "Cost this week", value: money(current.cost_usd), delta: delta(current.cost_usd, previous.cost_usd) },
    { label: "Merges", value: String(current.merges), delta: delta(current.merges, previous.merges) },
    { label: "Cost per merge", value: money(perMerge(current)), delta: delta(perMerge(current), perMerge(previous)) },
  ];
}

const plural = (/** @type {number} */ count, /** @type {string} */ word) => (count ? `${count} ${word}${count === 1 ? "" : "s"}` : `no ${word}`);
// A `day` is a local date with no zone, so it is read and written as UTC to stay the same date.
const dayName = (/** @type {string} */ day, /** @type {Intl.DateTimeFormatOptions} */ options) => new Date(`${day}T00:00:00Z`).toLocaleDateString("en", { timeZone: "UTC", ...options });

/** One bar per day, scaled to the busiest day, with a sentence for screen readers. @param {any} costs */
export function dayBars(costs) {
  // The window starts at the daemon's local midnight, so 12 hours later is still that local date in UTC.
  // shortcut: wrong for a daemon at UTC+12 or beyond; send the start's local date from the server if one runs there.
  const first = new Date(Date.parse(costs.current.since) + 12 * 3600000).toISOString().slice(0, 10);
  const max = Math.max(0, ...costs.days.map((/** @type {any} */ day) => day.cost_usd ?? 0));
  return costs.days.map((/** @type {any} */ day) => {
    const cost = day.cost_usd == null ? "no priced tokens" : money(day.cost_usd);
    return {
      day: day.day,
      weekday: dayName(day.day, { weekday: "short" }),
      date: String(Number(day.day.slice(8))),
      fraction: max ? (day.cost_usd ?? 0) / max : 0,
      merges: day.merges,
      current: day.day >= first,
      text: `${dayName(day.day, { weekday: "short", month: "short", day: "numeric" })}: ${cost}, ${plural(day.merges, "merge")}`,
    };
  });
}

export const SPLITS = ["workflow", "agent", "model", "machine", "repo"];
const count = new Intl.NumberFormat("en");
const tokenCount = (/** @type {any} */ tokens) => count.format(tokens.input + tokens.output + tokens.cache_read + tokens.cache_write);
const records = (/** @type {number} */ n) => `${n} record${n === 1 ? "" : "s"}`;

/** The current window split by `costs.by`, and what in it has no cost. @param {any} costs */
export function splitTable(costs) {
  const { by, current } = costs;
  return {
    dimension: by[0].toUpperCase() + by.slice(1),
    rows: costs.split.map((/** @type {any} */ row) => ({
      name: row[by] ?? "-",
      cost: money(row.cost_usd),
      tokens: tokenCount(row.tokens),
      note: [row.unpriced.length ? "price missing" : "", row.unknown ? `tokens unknown for ${records(row.unknown)}` : ""].filter(Boolean).join(", "),
    })),
    unpriced: current.unpriced.map((/** @type {any} */ entry) => ({ model: entry.model, tokens: tokenCount(entry.tokens) })),
    unknown: current.unknown ? `${records(current.unknown)} ${current.unknown === 1 ? "has" : "have"} no tokens` : null,
  };
}

const PARTS = [
  ["working", "Working", "working_minutes"],
  ["person", "Waiting on you", "person_wait_minutes"],
  ["world", "Waiting on the world", "world_wait_minutes"],
  ["queued", "Queued", "queued_minutes"],
];

/** Minutes as "12 min" or "1 h 5 min"; "-" with no value. @param {number | null} value */
function minutes(value) {
  if (value == null) return "-";
  const whole = Math.round(value);
  if (whole < 60) return `${whole} min`;
  if (whole >= 1440) {
    const hours = Math.floor((whole % 1440) / 60);
    return `${Math.floor(whole / 1440)} d${hours ? ` ${hours} h` : ""}`;
  }
  const rest = whole % 60;
  return `${Math.floor(whole / 60)} h${rest ? ` ${rest} min` : ""}`;
}

/** The person's attention: minutes per merge, ask waits, and each task's time split. @param {any} report */
export function attentionView(report) {
  const { waits } = report;
  const present = (/** @type {number | null} */ value) => (value == null ? "" : `${minutes(value)} while you were present`);
  return {
    headline: {
      label: "minutes of your attention per merge",
      value: report.person_minutes_per_merge == null ? "-" : String(Math.round(report.person_minutes_per_merge)),
      note: `${minutes(report.person_minutes)}${report.merges ? ` over ${plural(report.merges, "merge")}` : ", no merge yet"}, ${plural(report.acts, "act")}`,
    },
    waits: [
      { label: "Ask wait, median", value: minutes(waits.median_minutes), note: present(waits.median_present_minutes) },
      { label: "Ask wait, p90", value: minutes(waits.p90_minutes), note: present(waits.p90_present_minutes) },
      { label: "Asks acted on", value: String(waits.acted), note: waits.open ? `${waits.open} still open` : "" },
    ],
    tasks: report.tasks.map((/** @type {any} */ task) => {
      const parts = PARTS.map(([part, label, field]) => ({
        part, label, fraction: task.wall_minutes ? task[field] / task.wall_minutes : 0, value: minutes(task[field]),
      }));
      return {
        id: task.id, key: task.key, workflow: task.workflow, state: task.state, wall: minutes(task.wall_minutes), parts,
        text: `Task ${task.key}, ${minutes(task.wall_minutes)}: ${parts.map((each) => `${each.label.toLowerCase()} ${each.value}`).join(", ")}`,
      };
    }),
  };
}
