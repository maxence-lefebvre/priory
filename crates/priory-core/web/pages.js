// What each page is called: its panel's label, its title, and where its back link goes.
import { taskLabel } from "./tasks.js";

/** Each panel's label and icon. */
export const PANELS = {
  desk: ["Desk", "desk"], asks: ["Asks", "asks"], tasks: ["Tasks", "tasks"], runs: ["Runs", "runs"],
  inbox: ["Inbox", "inbox"], reports: ["Reports", "reports"], queue: ["Queue", "queue"],
  terminal: ["Terminal", "terminal"], workflow: ["Flows", "workflow"], costs: ["Costs", "costs"],
};


/** The parts of a path, `/` and `/index.html` being the Desk's. @param {string} url */
const parts = (url) => {
  const [first, ...rest] = url.split(/[?#]/)[0].split("/").filter(Boolean);
  return [!first || first === "index.html" ? "desk" : first, ...rest];
};

/** The panel a list URL such as `/queue?view=held` shows, else null. @param {string} url */
const panelOf = (url) => {
  const [first, id] = parts(url);
  return !id && Object.hasOwn(PANELS, first) ? first : null;
};

/**
 * A detail page's back link, `[href, label]`: the list `from` that the person left, or Tasks.
 * @param {string | null} from
 */
export function backTo(from) {
  const panel = from && panelOf(from);
  return panel ? [from, PANELS[panel][0]] : ["/tasks", "Tasks"];
}

/** The text fields a repaint gives back their focus, caret and selection, by id. */
export const TYPED_FIELDS = ["draft", "say", "report-draft", "report-query", "term-line", "term-keys"];

/** A part of an address as text, or as it is written when it does not decode. */
export function decoded(part) {
  try {
    return decodeURIComponent(part);
  } catch {
    return part;
  }
}

/**
 * The document title of the page at `path`: a task by its key, an ask by its number.
 * @param {string} path @param {{ tasks: any[] }} data
 */
export function pageTitle(path, data) {
  const [first, id] = parts(path);
  const number = /^\d+$/.test(id || "");
  let name = "Page not found";
  if (first === "tasks" && number) name = taskLabel(data.tasks.find((task) => String(task.id) === id)?.key) || `Task ${id}`;
  else if (first === "asks" && id) name = `Ask #${decoded(id)}`;
  else if (first === "batches" && number) name = `Batch ${id}`;
  else if (first === "memories") name = "Memory candidates";
  else if (first === "more") name = "More";
  else if (first === "settings") name = id === "notifications" ? "Notifications" : "Appearance";
  else if (Object.hasOwn(PANELS, first)) name = PANELS[first][0];
  return `${name} · Priory`;
}

/**
 * The read of `/api/tasks/{id}` an ask's drawer or a task page draws from. A refresh marks the kept
 * read stale: it keeps its body on screen while the task is read again, so no repaint draws the page
 * without it. Another task starts empty. `read` says whether to start a new read.
 * @param {{ id: any, body: any, stale?: boolean } | null} kept @param {any} id
 */
export function taskRead(kept, id) {
  const same = kept?.id === id ? kept : null;
  if (same && !same.stale) return { task: same, read: false };
  return { task: { id, body: same?.body ?? null, error: null, stale: false }, read: true };
}
