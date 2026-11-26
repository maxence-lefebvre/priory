import { useEffect } from "react";
import { askFor, asked } from "../legacy/data.js";

/** Reads `listing` without its older rows again once the part on screen no longer shows them, as a legacy page's open does. */
export function useOlder(listing, wanted) {
  useEffect(() => {
    const { all } = asked.get();
    if (!wanted && all[listing]) askFor({ all: { ...all, [listing]: false } });
  }, [listing, wanted]);
}
