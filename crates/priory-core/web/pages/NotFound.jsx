import { memo } from "react";
import { usePlace } from "./place.js";

export const NotFound = memo(function NotFound() {
  const body = <><div className="top"><h1>Page not found</h1></div><div className="scroll"><div className="empty notfound"><p>No page lives at this address.</p><a className="btn secondary" href="/">Open the Desk</a></div></div></>;
  return usePlace().size === "compact" ? body : <div className="list ">{body}</div>;
});
