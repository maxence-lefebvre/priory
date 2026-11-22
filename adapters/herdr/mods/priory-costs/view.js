// The view model of the web view's Costs panel: `/api/costs` and `/api/attention` in, stat tiles,
// bars and table rows out. No drawing, no IO. adapters/herdr/mods/priory-costs/view.js is a copy; a
// test keeps the two the same. Herdr's engine has no Intl, so numbers and dates format by hand.

const grouped = (/** @type {string} */ digits) => digits.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
export const money = (/** @type {number | null} */ value) => {
  if (value == null) return "-";
  // toPrecision drops the binary noise that makes 1.005 * 100 read 100.4999..., so half a cent rounds up.
  const cents = Math.round(Number((Math.abs(value) * 100).toPrecision(15)));
  return `${value < 0 ? "-" : ""}$${grouped(String(Math.floor(cents / 100)))}.${String(cents % 100).padStart(2, "0")}`;
};

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
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const weekday = (/** @type {string} */ day) => WEEKDAYS[new Date(`${day}T00:00:00Z`).getUTCDay()];
const longDay = (/** @type {string} */ day) => `${weekday(day)}, ${MONTHS[Number(day.slice(5, 7)) - 1]} ${Number(day.slice(8))}`;

/** One bar per day, scaled to the busiest day, with a sentence for screen readers. @param {any} costs */
export function dayBars(costs) {
  // The daemon names the local date its window starts on, in its own zone, as it keys the days.
  const first = costs.current.since_day;
  const max = Math.max(0, ...costs.days.map((/** @type {any} */ day) => day.cost_usd ?? 0));
  return costs.days.map((/** @type {any} */ day) => {
    const cost = day.cost_usd == null ? "no priced tokens" : money(day.cost_usd);
    return {
      day: day.day,
      weekday: weekday(day.day),
      date: String(Number(day.day.slice(8))),
      fraction: max ? (day.cost_usd ?? 0) / max : 0,
      merges: day.merges,
      current: day.day >= first,
      text: `${longDay(day.day)}: ${cost}, ${plural(day.merges, "merge")}`,
    };
  });
}

export const SPLITS = ["workflow", "agent", "model", "machine", "repo", "lane"];
const tokenCount = (/** @type {any} */ tokens) => grouped(String(tokens.input + tokens.output + tokens.cache_read + tokens.cache_write));
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

/** Each scorer's verdicts in the current window, by `costs.by`. An error means the scorer broke, so the rate leaves it out. @param {any} costs */
export function scoreTable(costs) {
  const { by } = costs;
  return {
    dimension: by[0].toUpperCase() + by.slice(1),
    rows: (costs.scores ?? []).map((/** @type {any} */ row) => {
      const graded = row.pass + row.fail;
      const rate = graded ? `${Math.round((row.pass / graded) * 100)}%` : "-";
      const counts = [[row.pass, "pass"], [row.fail, "fail"], [row.error, "error"], [row.not_applicable, "not applicable"]]
        .filter(([count]) => count).map(([count, word]) => `${count} ${word}`);
      return {
        name: row[by] ?? "-", scorer: row.scorer, rate, pass: row.pass, fail: row.fail, error: row.error, notApplicable: row.not_applicable,
        line: [graded ? `pass rate ${rate}` : "no pass rate", ...counts].join(", "),
      };
    }),
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
