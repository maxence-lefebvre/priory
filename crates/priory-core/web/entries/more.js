import { morePage } from "../legacy/settings.js";
import { html } from "../legacy/shared.js";

export const more = {
  name: "more",
  page: (size) => (size === "compact" ? morePage() : html`<div class="list ">${morePage()}</div>`),
};
