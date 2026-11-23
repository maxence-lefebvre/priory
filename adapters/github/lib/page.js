// Where the extension draws on GitHub's pull request pages: the old page and the new preview.

const OLD_NAV = 'nav[aria-label="Pull request navigation"], .tabnav-pr, .tabnav';
// The new page's tab row holds links to the pull request's other tabs.
const TAB_LINK = 'a[href*="/pull/"][href$="/commits"], a[href*="/pull/"][href$="/checks"], a[href*="/pull/"][href$="/files"], a[href*="/pull/"][href$="/changes"]';
const ROW = '[role="tablist"], nav, [role="navigation"]';

/** The pull request's tab row, with the page it is on, or null. @param {Document} doc */
export function anchor(doc) {
  const old = doc.querySelector(OLD_NAV);
  if (old) return { nav: old, kind: "old" };
  const nav = doc.querySelector(TAB_LINK)?.closest(ROW);
  return nav ? { nav, kind: "new" } : null;
}

/** Where the strip goes when no tab row is found. @param {Document} doc */
export const fallback = (doc) => doc.querySelector("main") ?? doc.body;

/** A tab of the row to copy for the Guide tab, or null. @param {Element} nav */
export const tabLike = (nav) => nav.querySelector('[role="tab"]') ?? nav.querySelector("a");

/** The description's rendered body, on either page, or null. @param {Document} doc */
export const descriptionBody = (doc) => doc.querySelector(".js-comment-body, .comment-body");

// The description holds the block as GitHub renders it: a <details> titled "Review guide"
// around the JSON. GitHub drops the HTML comment markers from the page.
/** The review guide's JSON in a pull request page, or null. @param {Document} doc */
export function blockIn(doc) {
  const body = descriptionBody(doc);
  const details = [...(body?.querySelectorAll("details") ?? [])].find((element) => element.querySelector("summary")?.textContent.trim() === "Review guide");
  return details?.querySelector("pre")?.textContent ?? null;
}
