import { rowsList, rowsPage } from "../legacy/frame.js";
import { current, listView } from "../legacy/shared.js";
import { tasksBody, tasksClick } from "../legacy/tasks.js";
import { TaskPage } from "../pages/TaskPage.jsx";
import { TASK_VIEWS } from "../tasks.js";

export const tasks = {
  name: "tasks",
  needs: ["tasks"],
  list: (wide) => rowsList("Tasks", tasksBody(wide)),
  page: () => rowsPage("Tasks", tasksBody(false)),
  Detail: TaskPage,
  older: { tasks: () => current() === "tasks" && listView(TASK_VIEWS) === "ended" },
  click: tasksClick,
};
