// The routes, in code. A legacy page is a child of `legacy`, which renders LegacyPage for all of them,
// so it stays mounted from one to the next. A port moves its route to the root, with its own component.
import { createRootRoute, createRoute, createRouter } from "@tanstack/react-router";
import { LegacyPage } from "./pages/LegacyPage.jsx";
import { NotFound } from "./pages/NotFound.jsx";
import { Shell } from "./pages/Shell.jsx";
import { TaskRoute } from "./pages/TaskPage.jsx";
import { history, hrefOf, parseSearch, stringifySearch } from "./router.js";

const root = createRootRoute({ component: Shell, notFoundComponent: NotFound });
const legacy = createRoute({ getParentRoute: () => root, id: "legacy", component: LegacyPage });

// Only a page's first segment picks it, as before the router: `/runs/x` is Runs.
const LEGACY = ["asks", "tasks", "runs", "inbox", "reports", "queue", "terminal", "workflow", "costs", "more", "memories", "settings"];
const legacyRoutes = ["/", "index.html", ...LEGACY.flatMap((page) => [page, `${page}/$`]), "batches/$batch"].filter((path) => path !== "tasks/$")
  .map((path) => createRoute({ getParentRoute: () => legacy, path }));

const task = createRoute({ getParentRoute: () => root, path: "tasks/$", component: TaskRoute });

export const makeRouter = (from = history) => createRouter({
  routeTree: root.addChildren([legacy.addChildren(legacyRoutes), task]),
  history: from,
  parseSearch,
  stringifySearch,
  trailingSlash: "preserve",
  scrollRestoration: true,
  // A page keeps its scroll by its address, as a person left it.
  getScrollRestorationKey: (location) => hrefOf(location).split("#")[0],
});
