import { state } from "../legacy/state.js";
import { taskClick, taskInput, taskSubmit } from "../legacy/taskacts.js";

// A task's actions, which several pages draw.
export const taskacts = {
  name: "taskacts",
  fields: ["say"],
  click: taskClick,
  submit: taskSubmit,
  input: taskInput,
  drain: (element) => (element.closest(".act") ? state.taskArmed : undefined),
};
