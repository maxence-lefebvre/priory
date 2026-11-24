// The React shell: it owns the root, the route, the server data's cache and the events long poll.
// Pages not ported yet draw inside LegacyPage, with app.js.
import { useEffect, useLayoutEffect, useSyncExternalStore } from "react";
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { QueryClientProvider } from "@tanstack/react-query";
import * as legacy from "./app.js";
import { follow } from "./events.js";
import { followBack, route } from "./router.js";

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// React unmounts the root when an effect throws, so a failed draw is reported and the page stays.
function draw() {
  try {
    legacy.render();
  } catch (error) {
    reportError(error);
  }
}

// app.js draws #app with innerHTML. React renders the element once and never its children.
function LegacyPage({ visit }) {
  useLayoutEffect(draw, [visit]);
  return <div id="app" />;
}

function Shell() {
  const visit = useSyncExternalStore(route.subscribe, route.get);
  useEffect(followBack, []);
  useEffect(() => {
    let live = true;
    legacy.refresh();
    follow({
      live: () => live,
      ready: () => (document.hidden ? undefined : legacy.revision()),
      held: legacy.revision,
      wait: (since) => legacy.api(`/api/events?since=${since}`),
      moved: legacy.revisionMoved,
      refresh: legacy.refresh,
      sleep,
    });
    return () => { live = false; };
  }, []);
  return <LegacyPage visit={visit} />;
}

// The desk can take seconds: the kept listing or the loading line shows before this returns.
flushSync(() => createRoot(document.getElementById("root")).render(
  <QueryClientProvider client={legacy.cache}>
    <Shell />
  </QueryClientProvider>,
));
