import { useLayoutEffect, useRef } from "react";

// Each scroll a person left, by the place it was left at.
const kept = new Map();

/** A ref for a `.scroll` element that keeps its scroll under `key`, such as its address, once `ready` draws it. */
export function useKeptScroll(key, ready = true) {
  const ref = useRef(null);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return undefined;
    element.scrollTop = kept.get(key) ?? 0;
    const keep = () => kept.set(key, element.scrollTop);
    element.addEventListener("scroll", keep, { passive: true });
    return () => element.removeEventListener("scroll", keep);
  }, [key, ready]);
  return ref;
}
