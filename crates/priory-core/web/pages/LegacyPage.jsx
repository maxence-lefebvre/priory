// A page not ported yet: app.js draws it with innerHTML, inside the React frame.
import { memo, useLayoutEffect, useRef } from "react";
import * as legacy from "../app.js";
import { asked, land } from "../legacy/data.js";
import { useListings, useStore } from "../hooks/useData.js";
import { useMinute } from "../hooks/useScreen.js";
import { usePlace } from "./place.js";

const FRAME = ["views", "links", "desk"];

// React renders the element once and never its children. CSS gives it no box of its own.
export const LegacyPage = memo(function LegacyPage() {
  const { href, entry, trail, size, names } = usePlace();
  // The frame's listings too: the page lands, and reads what it alone reads, once they all did.
  const { reading, read } = useListings([...FRAME, ...names], useStore(asked));
  const minute = useMinute();
  const host = useRef(null);
  useLayoutEffect(() => legacy.mount(host.current), []);
  useLayoutEffect(() => legacy.open({ href, entry, trail, size, names }), [href, entry, trail, size, names]);
  useLayoutEffect(() => {
    if (!reading) land();
  }, [reading, read, minute]);
  return <div className="legacy" ref={host} />;
});
