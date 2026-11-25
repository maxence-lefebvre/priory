// The routes, in code. Every page is a child of one route that renders the grid, so a slot stays
// mounted from one page to the next. Which part fills a slot is its entry's, under entries/.
import { createRootRoute, createRoute, createRouter } from "@tanstack/react-router";
import { NotFound } from "./pages/NotFound.jsx";
import { PageGrid } from "./pages/PageGrid.jsx";
import { Shell } from "./pages/Shell.jsx";
import { history, hrefOf, parseSearch, stringifySearch } from "./router.js";

const root = createRootRoute({ component: Shell, notFoundComponent: NotFound });
const page = createRoute({ getParentRoute: () => root, id: "page", component: PageGrid });

// Only a page's first segment picks it, as before the router: `/runs/x` is Runs.
const PAGES = ["asks", "tasks", "runs", "inbox", "reports", "queue", "terminal", "workflow", "costs", "more", "memories", "settings"];
const pageRoutes = ["/", "index.html", ...PAGES.flatMap((name) => [name, `${name}/$`]), "batches/$batch"]
  .map((path) => createRoute({ getParentRoute: () => page, path }));

export const makeRouter = (from = history) => createRouter({
  routeTree: root.addChildren([page.addChildren(pageRoutes)]),
  history: from,
  parseSearch,
  stringifySearch,
  trailingSlash: "preserve",
  scrollRestoration: true,
  // A page keeps its scroll by its address, as a person left it.
  getScrollRestorationKey: (location) => hrefOf(location).split("#")[0],
});
