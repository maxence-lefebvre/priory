// The React app: it owns the root, the router, the server data's cache and the events long poll.
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { cache, refresh, reread, revision } from "./data.js";
import "./entries/index.js";
import { follow, recover } from "./events.js";
import { watch } from "./components/TerminalPane.jsx";
import { staleAll, stuck } from "./queries.js";
import { followLink } from "./router.js";
import { makeRouter } from "./routes.jsx";
import { api, keepTokenForWorker, session } from "./session.js";
import { shown } from "./shown.js";

const { token } = session.get();
if (token) keepTokenForWorker(token);
document.addEventListener("click", followLink);

document.addEventListener("visibilitychange", () => {
  if (document.hidden || shown.get()) {
    shown.set(!document.hidden);
    if (!document.hidden) refresh();
    return;
  }
  // The mounted queries read once, as the shown tab enables them.
  staleAll(cache);
  shown.set(true);
});

// Ages move and a snooze ends with no new revision, so what is on screen reads again once a minute.
setInterval(() => { if (shown.get()) reread(); }, 60000);

const live = () => true;
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
follow({
  live,
  ready: () => (shown.get() ? revision() : undefined),
  held: revision,
  wait: (since) => api(`/api/events?since=${since}`),
  moved: () => watch.revision(),
  refresh: reread,
  sleep,
});
recover({ live, may: () => shown.get() && Boolean(session.get().token), stuck: () => stuck(cache), refresh: reread, sleep });

if ("serviceWorker" in navigator) navigator.serviceWorker.register("/sw.js").catch(() => {});

// The page keeps the id the earlier app.js draws in: an offline start or a restart mid-load can pair
// an old index with a new bundle, or the reverse, and neither may draw a blank page.
const root = document.getElementById("app");
root.id = "root";

const router = makeRouter();
// The first page's route is known before the first draw, so the kept data or the loading line shows at once.
router.load();
flushSync(() => createRoot(root).render(
  <QueryClientProvider client={cache}>
    <RouterProvider router={router} />
  </QueryClientProvider>,
));
