// What a list's page draws on a phone: its top, then its panel.
import { memo } from "react";
import { Connection } from "./Frame.jsx";

export const PageTop = memo(function PageTop({ title }) {
  return <div className="top"><h1>{title}</h1><span className="sp" /><Connection /></div>;
});

/** `kind` is a class beside `panel`, such as Reports' own. `rest` holds a data attribute app.css matches. */
export const Panel = memo(function Panel({ label, kind, children, ...rest }) {
  return <div className={kind ? `panel ${kind}` : "panel"} role="region" aria-label={label} {...rest}>{children}</div>;
});
