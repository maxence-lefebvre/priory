// The terminal pane inside a React page: React draws an empty box and legacy/terminal.js fills it.
import { useLayoutEffect } from "react";
import { keyboard, paintTerminal, syncTerminal, terminal, watch } from "../legacy/terminal.js";

/**
 * Paints the pane of the box `ref` holds, and polls its screen while the box is on screen and the tab
 * shown. Fit follows a resize. Another run starts over. Leaving, or the page going away, stops the
 * polling and gives a held keyboard back, so the run's outbox does not wait out the 15 minutes.
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
    const shownAgain = () => syncTerminal(page);
    const resized = () => { if (terminal.fit) paintTerminal(page); };
    const gone = () => keyboard.leave();
    document.addEventListener("visibilitychange", shownAgain);
    addEventListener("resize", resized);
    addEventListener("pagehide", gone);
    return () => {
      observer.disconnect();
      document.removeEventListener("visibilitychange", shownAgain);
      removeEventListener("resize", resized);
      removeEventListener("pagehide", gone);
      watch.hide();
      keyboard.leave();
    };
  }, [ref, run, end]);
}
