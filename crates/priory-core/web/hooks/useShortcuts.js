import { useEffect } from "react";
import { openAdd } from "../components/AddSheet.jsx";
import { first } from "../registry.js";
import { go } from "../router.js";
import { shortcut } from "../shortcuts.js";

// The asks the list draws.
const drawnRows = () => [...document.querySelectorAll(".list [href^='/asks/'], .list tr[data-href^='/asks/']")]
  .map((row) => ({ href: row.getAttribute("href") || row.dataset.href, current: row.getAttribute("aria-current") === "true" }));

/** Listens for the shortcuts while the person is signed in. */
export function useShortcuts({ ask }, listUrl) {
  useEffect(() => {
    const press = (event) => {
      if (document.querySelector("dialog[open]") || event.target.closest("input, textarea, select") || event.metaKey || event.ctrlKey || event.altKey) return;
      shortcut(event, { at: { ask, listUrl }, rows: drawnRows(), onBody: document.activeElement === document.body, go, add: openAdd, keys: (pressed) => first("key", pressed) });
    };
    document.addEventListener("keydown", press);
    return () => document.removeEventListener("keydown", press);
  }, [ask, listUrl]);
}
