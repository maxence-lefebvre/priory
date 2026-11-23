// The React shell: it owns the root, the route, the server data and the events long poll. Pages not
// ported yet draw inside LegacyPage, with app.js.
import { useEffect, useLayoutEffect, useSyncExternalStore } from "react";
import { createRoot } from "react-dom/client";
import { QueryClientProvider, useQueries } from "@tanstack/react-query";
import * as legacy from "./app.js";
import { follow } from "./events.js";
import { observed } from "./queries.js";
import { followBack, route } from "./router.js";

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// app.js draws #app with innerHTML. React renders the element once and never its children.
function LegacyPage({ at }) {
  useLayoutEffect(() => legacy.render(), [at]);
  return <div id="app" />;
}

function Shell() {
  const at = useSyncExternalStore(route.subscribe, route.get);
  const wanted = useSyncExternalStore(legacy.wanted.subscribe, legacy.wanted.get);
  const signedIn = useSyncExternalStore(legacy.signedIn.subscribe, legacy.signedIn.get);
  const query = observed(legacy.api, wanted);
  const listings = useQueries({ queries: query.queries.map((each) => ({ ...each, enabled: signedIn })), combine: query.combine });
  useLayoutEffect(() => { if (signedIn && listings) legacy.accept(listings); }, [signedIn, listings]);
  useEffect(followBack, []);
  useEffect(() => {
    let live = true;
    legacy.refresh();
    follow({
      live: () => live,
      ready: () => (legacy.signedIn.get() && !document.hidden ? legacy.revision() : undefined),
      wait: (since) => legacy.api(`/api/events?since=${since}`),
      moved: legacy.revisionMoved,
      refresh: legacy.refresh,
      sleep,
    });
    return () => { live = false; };
  }, []);
  return <LegacyPage at={at} />;
}

createRoot(document.getElementById("root")).render(
  <QueryClientProvider client={legacy.cache}>
    <Shell />
  </QueryClientProvider>,
);
