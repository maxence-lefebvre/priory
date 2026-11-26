// The Terminal panel is React. The events are the task page's terminal pane's, which legacy/terminal.js paints.
import { keyboard, terminalClick, terminalInput, terminalKey, terminalSubmit, watch } from "../legacy/terminal.js";
import { TerminalList, TerminalPage } from "../pages/Terminal.jsx";

export const terminal = {
  name: "terminal",
  List: TerminalList,
  Page: TerminalPage,
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
