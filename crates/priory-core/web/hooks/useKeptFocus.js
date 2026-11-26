import { useLayoutEffect } from "react";

// The control a part had focused as it left, by its id or address.
let lost = null;

/** Gives focus back to the control a part had focused, once the part draws again in another place, as a resize moves a panel. */
export function useKeptFocus(ref) {
  useLayoutEffect(() => {
    const element = ref.current;
    if (lost && element && document.activeElement === document.body) element.querySelector(lost)?.focus({ preventScroll: true });
    lost = null;
    return () => {
      const active = document.activeElement;
      if (!element?.contains(active)) return;
      const href = active.getAttribute("href");
      lost = active.id ? `#${CSS.escape(active.id)}` : href ? `[href="${CSS.escape(href)}"]` : null;
    };
  }, [ref]);
}
