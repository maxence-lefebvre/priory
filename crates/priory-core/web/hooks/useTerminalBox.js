// The terminal pane inside a React page: React draws an empty box and legacy/terminal.js fills it.
import { useLayoutEffect } from "react";
import { keyboard, paintTerminal, syncTerminal, terminal, watch } from "../legacy/terminal.js";

/**
 * Paints the pane of the box `ref` holds, and polls its screen while the box is on screen. Another
 * run starts over. Leaving stops the polling and gives a held keyboard back.
 */
export function useTerminalBox(ref, run, end) {
  useLayoutEffect(() => {
    const box = ref.current;
    if (!box) return undefined;
    const page = box.parentElement;
    paintTerminal(page, true);
    terminal.onScreen = false;
    const observer = new IntersectionObserver(([entry]) => {
      terminal.onScreen = entry.isIntersecting;
      syncTerminal(page);
    });
    observer.observe(box);
    return () => {
      observer.disconnect();
      watch.hide();
      keyboard.leave();
    };
  }, [ref, run, end]);
}
