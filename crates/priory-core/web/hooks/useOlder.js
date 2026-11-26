import { useEffect } from "react";
import { askFor, asked } from "../data.js";

/** Reads `listing` without its older rows again once the part on screen no longer shows them. */
export function useOlder(listing, wanted) {
  useEffect(() => {
    const { all } = asked.get();
    if (!wanted && all[listing]) askFor({ all: { ...all, [listing]: false } });
  }, [listing, wanted]);
}
