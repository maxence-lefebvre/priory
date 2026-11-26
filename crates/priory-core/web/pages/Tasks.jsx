// Tasks (DESIGN.md, Tasks panel): every task, by saved view and facets; each row opens its page.
import { memo, useEffect } from "react";
import { TASK_VIEWS, filtered, olderPart, remoteId, taskFacets, taskViews } from "../tasks.js";
import { askFor, asked } from "../legacy/data.js";
import { Facets, OlderRow, PageLoading, RowsList, RowsPage, facetPicks, facetQuery, listView } from "../components/Lists.jsx";
import { SavedViews } from "../components/SavedViews.jsx";
import { TaskListRow, TaskTableRow } from "../components/TaskRows.jsx";
import { useStore, useTasks } from "../hooks/useData.js";
import { usePlace } from "./place.js";

const EMPTY = {
  active: <>No active tasks. Add one with the + button, or run <code>priory add</code>.</>,
  blocked: "No blocked tasks. A task shows here when it needs a person to go on.",
  ended: "No task ended in the last 7 days. A task shows here once it is done, failed or dropped.",
};
const keyOf = (task) => task.id ?? remoteId(task);

// The tasks listing, with its ended tasks older than 7 days once the person scrolled to them.
function useTaskList() {
  const wanted = useStore(asked);
  const { href } = usePlace();
  const current = listView(href, TASK_VIEWS);
  // Only the Ended view shows the older rows: another view reads without them.
  useEffect(() => {
    if (current !== "ended" && wanted.all.tasks) askFor({ all: { ...wanted.all, tasks: false } });
  }, [current, wanted]);
  return { ...useTasks(wanted), href, current };
}

function body({ data, href, current }, wide) {
  const all = data.tasks;
  const specs = taskFacets(all);
  const picks = facetPicks(href, specs);
  const views = taskViews(filtered(all, picks, specs));
  const { counted, unloaded } = olderPart({ view: current, history: "ended", older: data.older, picked: picks.size > 0 });
  const chips = [
    { key: "active", label: "Active", count: views.active.length, on: current === "active" },
    { key: "blocked", label: "Blocked", count: views.blocked.length, on: current === "blocked" },
    { key: "ended", label: "Ended", count: views.ended.length + counted, on: current === "ended" },
  ];
  const head = <><SavedViews views={chips} base="/tasks" facets={facetQuery(picks)} /><Facets view={current} picks={picks} specs={specs} base="/tasks" items={all} /></>;
  const list = views[current];
  if (!list.length && !unloaded) return { head, rows: <p className="empty">{EMPTY[current]}</p> };
  const rows = wide
    ? <table className="tbl tasks"><thead><tr><th>State</th><th>Task</th><th>Title</th><th>Workflow and step</th><th>Lane</th><th>Band</th><th className="r">Age</th></tr></thead><tbody>{list.map((task) => <TaskTableRow key={keyOf(task)} task={task} />)}<OlderRow listing="tasks" count={unloaded} wide columns={7} /></tbody></table>
    : <>{list.map((task) => <TaskListRow key={keyOf(task)} task={task} />)}<OlderRow listing="tasks" count={unloaded} /></>;
  return { head, rows };
}

/** Tasks beside other panels. */
export const TasksList = memo(function TasksList() {
  const read = useTaskList();
  const { size } = usePlace();
  if (!read.data) return <PageLoading error={read.error} />;
  const { head, rows } = body(read, size === "wide");
  return <RowsList label="Tasks" href={read.href} views={head}>{rows}</RowsList>;
});

/** Tasks alone on a phone. */
export const TasksPage = memo(function TasksPage() {
  const read = useTaskList();
  if (!read.data) return <PageLoading error={read.error} />;
  const { head, rows } = body(read, false);
  return <RowsPage label="Tasks" href={read.href} views={head}>{rows}</RowsPage>;
});
