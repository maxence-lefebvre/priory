// A panel of a tree that only has a page of its own, such as Costs beside the Desk.
import { memo } from "react";
import { PANELS } from "../pages.js";

/** The list part of `panel`, which says the panel comes later. */
export const later = (panel) => memo(function LaterList() {
  const [label] = PANELS[panel];
  return <div className="list" role="region" aria-label={label}><div className="lhead"><h1>{label}</h1></div><p className="empty">{`The ${label} panel comes in a later version of the web view. Meanwhile run `}<code />.</p></div>;
});
