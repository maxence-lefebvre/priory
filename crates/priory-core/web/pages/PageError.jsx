// What a page that failed to draw says (DESIGN.md, Error), in the frame, or alone when the frame failed.
import { memo } from "react";
import { listClass, usePlace } from "./place.js";

const Why = memo(function Why({ error }) {
  return (
    <>
      <div className="top"><h1>Could not draw this page</h1></div>
      <div className="scroll"><div className="empty notfound" role="alert"><p>{`The web view failed: ${error.message}. Retry, or open the Desk.`}</p><button type="button" className="btn secondary" onClick={() => window.location.reload()}>Retry</button><a className="btn secondary" href="/">Open the Desk</a></div></div>
    </>
  );
});

export const PageError = memo(function PageError({ error }) {
  return <div className={listClass(usePlace().size, "list ")}><Why error={error} /></div>;
});

export const RootError = memo(function RootError({ error }) {
  return <div id="app"><Why error={error} /></div>;
});
