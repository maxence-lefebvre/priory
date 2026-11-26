// What the lists of rows draw around their rows: the panel, the facets, the older rows and the loading line.
import { memo, useEffect, useLayoutEffect, useRef } from "react";
import { searchOf } from "../frame.js";
import { go } from "../router.js";
import { failureWords, session } from "../session.js";
import { useStore } from "../hooks/useData.js";
import { loadOlder } from "../legacy/shared.js";
import { Icon } from "./Icon.jsx";
import { PageTop, Panel } from "./PhonePage.jsx";
import { remoteClick } from "./TaskRows.jsx";

/** The saved view `href` names, else the list's first. */
export const listView = (href, views) => {
  const name = new URLSearchParams(searchOf(href)).get("view");
  return views.includes(name) ? name : views[0];
};

/** The facet values `href` picks, by key. */
export const facetPicks = (href, facets) => {
  const search = new URLSearchParams(searchOf(href));
  return new Map(facets.map(([key]) => [key, search.get(key)]).filter(([, value]) => value));
};
export const facetQuery = (picks) => [...picks].map(([key, value]) => `&${key}=${encodeURIComponent(value)}`).join("");

// A facet chip of the list at `base` over `items`; a facet with no order offers its values A to Z.
const Facet = memo(function Facet({ view, picks, spec: [key, label, values, order, named = (name) => name], base, items }) {
  const others = new Map([...picks].filter(([other]) => other !== key));
  const picked = picks.get(key);
  if (picked) return <a className="facet" aria-current="true" href={`${base}?view=${view}${facetQuery(others)}`} aria-label={`${label}: ${picked}. Clear`}>{picked}<Icon name="x" size="s13" /></a>;
  const counts = new Map();
  for (const item of items) for (const value of values(item)) counts.set(value, (counts.get(value) || 0) + 1);
  const offered = [...counts].sort(([a], [b]) => (order ? order().indexOf(a) - order().indexOf(b) : a.localeCompare(b)));
  return (
    <details className="facet"><summary aria-label={`Filter by ${key}`}>{label}<Icon name="chevron-down" size="s13" /></summary>
      <div className="pick">{offered.map(([name, count]) => <a key={name} href={`${base}?view=${view}${facetQuery(others)}&${key}=${encodeURIComponent(name)}`}>{named(name)}<span className="num">{count}</span></a>)}</div></details>
  );
});

export const Facets = memo(function Facets({ view, picks, specs, base, items }) {
  return <div className="facets" aria-label="Facets">{specs.map((spec) => <Facet key={spec[0]} view={view} picks={picks} spec={spec} base={base} items={items} />)}</div>;
});

/** The last row of a list that left out older rows; scrolling to it loads them. */
export function OlderRow({ listing, count, wide, columns = 6 }) {
  const row = useRef(null);
  useEffect(() => {
    if (!count) return undefined;
    const observer = new IntersectionObserver(([entry]) => { if (entry.isIntersecting) loadOlder(listing); });
    observer.observe(row.current);
    return () => observer.disconnect();
  }, [listing, count]);
  if (!count) return null;
  const text = `Loading older ${listing}…`;
  return wide ? <tr className="grp" data-older={listing} ref={row}><td colSpan={columns}>{text}</td></tr> : <p className="empty" data-older={listing} ref={row}>{text}</p>;
}

/** What a list says before its first read lands: loading, or why it could not read. */
export const PageLoading = memo(function PageLoading({ error }) {
  const { failed: down } = useStore(session);
  const failed = down ?? (error ? failureWords(error) : null);
  return failed
    ? <p className="empty">{`Could not load this page: ${failed}. Check that `}<code>priory serve</code> or the daemon runs, then retry.</p>
    : <p className="empty" role="status">Loading…</p>;
});

// A row's click opens its address; a link or control inside it, such as its key or Move menu, acts on
// its own. One listener on the list, as the rows have no role a handler of their own would need.
function clicked(event) {
  if (remoteClick(event)) return;
  const row = event.target.closest("[data-href]");
  if (row && !event.target.closest("a, button, details")) go(row.dataset.href);
}

// The scroll a person left a list at, kept by its address, so Back finds it again.
const scrolls = new Map();
function useScroller(href) {
  const scroller = useRef(null);
  useLayoutEffect(() => {
    const element = scroller.current;
    element.scrollTop = scrolls.get(href) ?? 0;
    const kept = () => scrolls.set(href, element.scrollTop);
    element.addEventListener("scroll", kept, { passive: true });
    element.addEventListener("click", clicked);
    return () => {
      element.removeEventListener("scroll", kept);
      element.removeEventListener("click", clicked);
    };
  }, [href]);
  return scroller;
}

/** A list panel of rows under its title, beside other panels. */
export function RowsList({ label, href, views, children }) {
  const scroller = useScroller(href);
  return <div className="list" role="region" aria-label={label}><div className="lhead"><h1>{label}</h1></div>{views}<div className="scroll" ref={scroller}>{children}</div></div>;
}

/** The same list alone on a phone. */
export function RowsPage({ label, href, views, children }) {
  const scroller = useScroller(href);
  return <><PageTop title={label} /><Panel label={label}><div className="scroll" ref={scroller}>{views}{children}</div></Panel></>;
}
