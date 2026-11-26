// What the legacy pages keep while the app runs. Each page adds its own part, in its own file.
import { acts } from "../acts.js";
import { session } from "../session.js";

// The place the page draws, set as it opens: never ahead of what React shows.
export const place = { href: "/", entry: {} };
export const trail = { list: "desk", listUrl: "/" };
export const shape = { size: "compact" };

export const state = {
  // The sign-in, the place and the actions live in plain modules; these read them.
  get token() { return session.get().token; },
  get failed() { return session.get().failed; },
  // The revision of the Desk the page drew.
  get revision() { return state.data?.desk.revision; },
  get list() { return trail.list; },
  get listUrl() { return trail.listUrl; },
  get taskArmed() { return acts.get().armed; },
  get taskBusy() { return acts.get().busy; },
  get taskNotices() { return acts.get().notices; },
  get sayDrafts() { return acts.get().drafts; },
  data: null,
  rendered: 0,
  scrolls: new Map(),
  observers: [],
};
