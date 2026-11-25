import { html } from "../legacy/shared.js";
import { keyboard, terminalClick, terminalInput, terminalKey, terminalSubmit, terminalsPanel, watch } from "../legacy/terminal.js";

export const terminal = {
  name: "terminal",
  needs: ["tasks"],
  list: () => html`<div class="list" role="region" aria-label="Terminal"><div class="lhead"><h1>Terminal</h1></div>${terminalsPanel()}</div>`,
  page: () => html`<div class="top"><h1>Terminal</h1></div>${terminalsPanel()}`,
  fields: ["term-line", "term-keys"],
  click: terminalClick,
  submit: terminalSubmit,
  input: terminalInput,
  keyFirst: terminalKey,
  signedOut() {
    watch.stop();
    // While the token still works: giving the keyboard back needs it.
    keyboard.leave();
  },
};
