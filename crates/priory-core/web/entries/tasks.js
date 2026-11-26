import { current, listView } from "../legacy/shared.js";
import { TaskPage } from "../pages/TaskPage.jsx";
import { TasksList, TasksPage } from "../pages/Tasks.jsx";
import { TASK_VIEWS } from "../tasks.js";

export const tasks = {
  name: "tasks",
  List: TasksList,
  Page: TasksPage,
  Detail: TaskPage,
  older: { tasks: () => current() === "tasks" && listView(TASK_VIEWS) === "ended" },
};
