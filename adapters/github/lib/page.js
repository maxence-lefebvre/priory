// Where the extension draws on GitHub's pull request pages: the classic page and the new one.
// The new page's class names change with GitHub's builds, so it is found by ids, ARIA and data-*.

const OLD_NAV = 'nav[aria-label="Pull request navigation"], .tabnav-pr, .tabnav';
const NEW_TAB = 'a[data-dd-action-name^="pull_requests.navigation."], a[data-telemetry-action^="pull_requests.navigation."], #prs-files-anchor-tab, #prs-commits-anchor-tab';
// A last resort: a row holding links to the pull request's other tabs.
const TAB_LINK = 'a[href*="/pull/"][href$="/commits"], a[href*="/pull/"][href$="/checks"], a[href*="/pull/"][href$="/files"], a[href*="/pull/"][href$="/changes"]';
const ROW = 'nav, [role="tablist"], [role="navigation"]';

/**
 * The pull request's tab row, where the strip goes before it (`at`), and which page it is, or null.
 * On the new page the strip goes before the React app, so React never meets a node it did not make.
 * @param {Document} doc
 */
export function anchor(doc) {
  const row = doc.querySelector(NEW_TAB)?.closest(ROW);
  if (row) return { nav: row, at: row.closest("react-app") ?? row, kind: "new" };
  const old = doc.querySelector(OLD_NAV);
  if (old) return { nav: old, at: old, kind: "old" };
  const nav = doc.querySelector(TAB_LINK)?.closest(ROW);
  return nav ? { nav, at: nav.closest("react-app") ?? nav, kind: "new" } : null;
}

/** Where the strip goes when no tab row is found: above GitHub's main, outside any React tree. @param {Document} doc */
export const fallback = (doc) => doc.querySelector(".application-main") ?? doc.body;

/** A tab of the row to copy for the Guide tab, one not selected when it can, or null. @param {Element} nav */
export const tabLike = (nav) =>
  [...nav.querySelectorAll(NEW_TAB)].find((tab) => !tab.hasAttribute("aria-current")) ?? nav.querySelector(NEW_TAB) ?? nav.querySelector('[role="tab"]') ?? nav.querySelector("a");

/** The description's rendered body, on either page, or null. @param {Document} doc */
export function descriptionBody(doc) {
  const old = doc.querySelector(".js-comment-body, .comment-body");
  if (old) return old;
  const first = [...doc.querySelectorAll('[data-testid="timeline-item"]')].find((item) => item.querySelector('[data-testid="comment-header"]'));
  return first?.querySelector(".markdown-body") ?? null;
}

// The description holds the block as GitHub renders it: a <details> titled "Review guide"
// around the JSON. GitHub drops the HTML comment markers from the page.
const guideIn = (body) =>
  [...(body?.querySelectorAll("details") ?? [])].find((element) => element.querySelector("summary")?.textContent.trim() === "Review guide")?.querySelector("pre")?.textContent ?? null;

/**
 * The review guide's JSON in a fetched pull request page, or null. Where the page renders the
 * description only in the browser, its HTML is a string in the page's embedded data.
 * @param {Document} doc
 */
export function blockIn(doc) {
  const found = guideIn(descriptionBody(doc));
  if (found) return found;
  for (const script of doc.querySelectorAll('script[type="application/json"][data-target$=".embeddedData"]')) {
    let data;
    try {
      data = JSON.parse(script.textContent);
    } catch {
      continue;
    }
    for (const html of strings(data)) {
      if (!html.includes("Review guide")) continue;
      const json = guideIn(new DOMParser().parseFromString(html, "text/html").body);
      if (json) return json;
    }
  }
  return null;
}

function* strings(value, depth = 0) {
  if (typeof value === "string") yield value;
  else if (value && typeof value === "object" && depth < 32) for (const child of Object.values(value)) yield* strings(child, depth + 1);
}
