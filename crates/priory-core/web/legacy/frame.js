// The legacy frame around a page.
import { countHead, sectionInView } from "../desk.js";
import { PANELS } from "../pages.js";
import { asksBody } from "./asks.js";
import { countLine, deskBody, nextCard } from "./desk.js";
import { inboxViews, unroutedRows } from "./inbox.js";
import { queueBody } from "./queue.js";
import { runsBody } from "./runs.js";
import { leaves, shownTab } from "../frame.js";
import { html, icon, loadOlder, state, track } from "./shared.js";
import { tasksBody } from "./tasks.js";
import { terminalsPanel } from "./terminal.js";


const CLI = { terminal: "priory attach" };

export function connection() {
  const off = Boolean(state.failed);
  return html`<span class="conn ${off ? "off" : ""}">${off ? "reconnecting" : "live"}</span>`;
}

export function laterPanel(panel) {
  if (panel === "terminal") return terminalsPanel();
  const [label] = PANELS[panel];
  return html`<p class="empty">The ${label} panel comes in a later version of the web view. Meanwhile run <code>${CLI[panel]}</code>.</p>`;
}

export function listPanel(panel, wide) {
  if (panel === "desk") {
    const { model, views, banner, rows } = deskBody(wide);
    const head = countHead(model.open.length);
    return html`<div class="list" role="region" aria-label="Desk"><div class="lhead"><h1>Desk</h1>
      <span class="cnt">${head.n ? html`<span class="n num">${head.n}</span>` : ""}<span class="w">${head.words}<span>${countLine(model)}</span></span></span></div>
      ${nextCard(model)}${views}${banner}<div class="scroll">${rows}</div>${wide ? keyHints() : ""}</div>`;
  }
  if (panel === "inbox") {
    return html`<div class="list inbox" role="region" aria-label="Inbox"><div class="lhead"><h1>Inbox</h1></div>${inboxViews()}<div class="scroll">${unroutedRows()}</div>${wide ? keyHints() : ""}</div>`;
  }
  if (panel === "asks") {
    const { views, rows } = asksBody(wide);
    return html`<div class="list" role="region" aria-label="Asks"><div class="lhead"><h1>Asks</h1></div>${views}<div class="scroll">${rows}</div>${wide ? keyHints() : ""}</div>`;
  }
  if (OWN_PAGE_LISTS[panel]) {
    const { views, rows } = OWN_PAGE_LISTS[panel](wide);
    return html`<div class="list" role="region" aria-label="${PANELS[panel][0]}"><div class="lhead"><h1>${PANELS[panel][0]}</h1></div>${views}<div class="scroll">${rows}</div></div>`;
  }
  return html`<div class="list" role="region" aria-label="${PANELS[panel][0]}"><div class="lhead"><h1>${PANELS[panel][0]}</h1></div>${laterPanel(panel)}</div>`;
}

// Lists whose rows open a page of their own, so they take the whole width, with no drawer.
export const OWN_PAGE_LISTS = { tasks: tasksBody, runs: runsBody, queue: queueBody };

const keyHints = () => html`<div class="keys"><span><kbd>J</kbd><kbd>K</kbd>move</span><span><kbd>Enter</kbd>open</span><span><kbd>Esc</kbd>close</span><span><kbd>N</kbd>add</span></div>`;

// The view file's tree, for a layout that is not a plain set of tabs.
export function tree(node, list) {
  if (node.panel) return listPanel(node.panel, true);
  const [kind, children] = Object.entries(node)[0];
  if (kind !== "tabs") return html`<div class="tree tree-${kind}">${children.map((child) => tree(child, list))}</div>`;
  const shown = shownTab(children, list);
  const strip = children.map((child) => {
    const panel = leaves(child)[0];
    return html`<a href="${panel === "desk" ? "/" : `/${panel}`}" aria-current="${child === shown ? "page" : "false"}">${icon(PANELS[panel][1], "s20")}${PANELS[panel][0]}</a>`;
  });
  return html`<div class="tree-tabs"><nav class="tabbar" aria-label="Tabs">${strip}</nav>${tree(shown, list)}</div>`;
}

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
