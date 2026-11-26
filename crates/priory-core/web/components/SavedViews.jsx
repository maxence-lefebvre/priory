import { memo } from "react";

/** The chips of a list's saved views, each { key, label, count, on }. `count` is null when the view has none. */
export const SavedViews = memo(function SavedViews({ views, base, facets = "" }) {
  return (
    <nav className="views" aria-label="Saved views">{views.map(({ key, label, count, on }) => (
      <a key={key} href={`${base}?view=${key}${facets}`} aria-current={on ? "true" : "false"}>{label}{count === null ? null : <span className="num">{count}</span>}</a>
    ))}</nav>
  );
});
