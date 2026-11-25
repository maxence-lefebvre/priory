import { memo } from "react";
import { listClass, usePlace } from "./place.js";

export const NotFound = memo(function NotFound() {
  return (
    <div className={listClass(usePlace().size, "list ")}>
      <div className="top"><h1>Page not found</h1></div>
      <div className="scroll"><div className="empty notfound"><p>No page lives at this address.</p><a className="btn secondary" href="/">Open the Desk</a></div></div>
    </div>
  );
});
