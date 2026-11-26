// What several legacy pages draw around their own part.
import { PANELS } from "../pages.js";
import { pageFailed } from "./data.js";
import { html, loadOlder, state, track } from "./shared.js";

/** A panel of a tree that only has a page of its own. */
export function laterList(panel) {
  const [label] = PANELS[panel];
  return html`<div class="list" role="region" aria-label="${label}"><div class="lhead"><h1>${label}</h1></div><p class="empty">The ${label} panel comes in a later version of the web view. Meanwhile run <code></code>.</p></div>`;
}

export const keyHints = () => html`<div class="keys"><span><kbd>J</kbd><kbd>K</kbd>move</span><span><kbd>Enter</kbd>open</span><span><kbd>Esc</kbd>close</span><span><kbd>N</kbd>add</span></div>`;

export function loading() {
  const failed = state.failed ?? pageFailed();
  return failed
    ? html`<p class="empty">Could not load this page: ${failed}. Check that <code>priory serve</code> or the daemon runs, then retry.</p>`
    : html`<p class="empty" role="status">Loading…</p>`;
}

// A list's last row loads its older rows once it scrolls into view.
export function watchOlder(app) {
  for (const row of app.querySelectorAll("[data-older]")) {
    track(new IntersectionObserver(([entry]) => { if (entry.isIntersecting) loadOlder(row.dataset.older); })).observe(row);
  }
}
