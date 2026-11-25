import { settingsChange, settingsClick, settingsKey, settingsScreen } from "../legacy/settings.js";
import { html, icon } from "../legacy/shared.js";

export const settings = {
  name: "settings",
  // The theme previews draw the Desk.
  needs: (section = "appearance") => (section === "notifications" ? [] : ["asks", "queue", "tasks"]),
  page: (size) => (size === "compact"
    ? html`<div class="top"><a class="back" href="/more">${icon("chevron-left", "s22")}More</a></div>${settingsScreen()}`
    : html`<div class="list ">${settingsScreen()}</div>`),
  click: settingsClick,
  change: settingsChange,
  keyFirst: settingsKey,
};
