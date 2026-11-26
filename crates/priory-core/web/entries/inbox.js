import { keyHints, rowsList, rowsPage } from "../legacy/frame.js";
import { inboxClick, inboxViews, loadUnrouted, unroutedRows } from "../legacy/inbox.js";
import { current } from "../legacy/shared.js";

const body = () => ({ views: inboxViews(), rows: unroutedRows() });

export const inbox = {
  name: "inbox",
  needs: ["tasks"],
  list: (wide) => rowsList("Inbox", body(), { hints: wide ? keyHints() : "", kind: "list inbox" }),
  page: () => rowsPage("Inbox", body()),
  click: inboxClick,
  landed: () => { if (current() === "inbox") loadUnrouted(); },
};
