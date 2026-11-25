import { rowsList, rowsPage } from "../legacy/frame.js";
import { queueBody, queueClick, queueKey } from "../legacy/queue.js";

export const queue = {
  name: "queue",
  needs: ["queue", "tasks"],
  list: (wide) => rowsList("Queue", queueBody(wide)),
  page: () => rowsPage("Queue", queueBody(false)),
  click: queueClick,
  key: queueKey,
};
