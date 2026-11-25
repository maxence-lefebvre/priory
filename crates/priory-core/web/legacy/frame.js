// What several legacy pages draw around their own part.
import { sectionInView } from "../desk.js";
import { PANELS } from "../pages.js";
import { html, loadOlder, state, track } from "./shared.js";

export function connection() {
  const off = Boolean(state.failed);
  return html`<span class="conn ${off ? "off" : ""}">${off ? "reconnecting" : "live"}</span>`;
}

/** A panel of a tree that only has a page of its own. */
export function laterList(panel) {
  const [label] = PANELS[panel];
  return html`<div class="list" role="region" aria-label="${label}"><div class="lhead"><h1>${label}</h1></div><p class="empty">The ${label} panel comes in a later version of the web view. Meanwhile run <code></code>.</p></div>`;
}

/** A list panel of rows under its title. */
export function rowsList(label, { views, rows }, { hints = "", kind = "list" } = {}) {
  return html`<div class="${kind}" role="region" aria-label="${label}"><div class="lhead"><h1>${label}</h1></div>${views}<div class="scroll">${rows}</div>${hints}</div>`;
}

/** The same list alone on a phone. */
export function rowsPage(label, { views, rows }) {
  return html`<div class="top"><h1>${label}</h1><span class="sp"></span>${connection()}</div>
    <div class="panel" role="region" aria-label="${label}"><div class="scroll">${views}${rows}</div></div>`;
}

export const keyHints = () => html`<div class="keys"><span><kbd>J</kbd><kbd>K</kbd>move</span><span><kbd>Enter</kbd>open</span><span><kbd>Esc</kbd>close</span><span><kbd>N</kbd>add</span></div>`;

export function loading() {
  return state.failed
    ? html`<p class="empty">Could not load this page: ${state.failed}. Check that <code>priory serve</code> or the daemon runs, then retry.</p>`
    : html`<p class="empty" role="status">Loading…</p>`;
}

// The section chips mark the section in view; a list's last row loads its older rows.
export function watchSections(app) {
  for (const row of app.querySelectorAll("[data-older]")) {
    track(new IntersectionObserver(([entry]) => { if (entry.isIntersecting) loadOlder(row.dataset.older); })).observe(row);
  }
  for (const scroller of app.querySelectorAll("[data-sections]")) {
    const chips = [...scroller.querySelectorAll(".jump a")];
    const sections = [...scroller.querySelectorAll(".sect")];
    let shown = null;
    const light = () => {
      const top = scroller.getBoundingClientRect().top - scroller.scrollTop;
      const tops = sections.map((section) => section.getBoundingClientRect().top - top);
      // A chip lands its section under the chip bar, at the section's scroll-margin-top.
      const bar = sections.length ? parseFloat(getComputedStyle(sections[0]).scrollMarginTop) || 0 : 0;
      const lit = sections[sectionInView({ tops, scrollTop: scroller.scrollTop, height: scroller.clientHeight, scrollHeight: scroller.scrollHeight, bar })];
      if (!lit || lit.id === shown) return;
      shown = lit.id;
      chips.forEach((chip) => chip.setAttribute("aria-current", String(chip.hash === `#${lit.id}`)));
      // On a phone the chips scroll sideways: keep the lit one in view.
      const chip = chips.find((each) => each.hash === `#${lit.id}`);
      const strip = chip?.parentElement;
      if (strip && strip.scrollWidth > strip.clientWidth) strip.scrollLeft = chip.offsetLeft - (strip.clientWidth - chip.offsetWidth) / 2;
    };
    if (chips.length) {
      scroller.addEventListener("scroll", light, { passive: true });
      light();
    }
    const options = scroller.querySelector(".opts");
    const bar = app.querySelector(".abar");
    const head = app.querySelector(".ahead:has(.aq)");
    if (!options || !(bar || head)) continue;
    track(new IntersectionObserver(([entry]) => {
      if (bar) bar.hidden = entry.isIntersecting;
      // Only options scrolled past shrink the header; options still below the fold leave it whole.
      head?.classList.toggle("shrunk", !entry.isIntersecting && entry.boundingClientRect.bottom <= (entry.rootBounds?.top ?? 0));
    }, { root: scroller })).observe(options);
  }
}
