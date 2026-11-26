// The Terminal panel: each running task, whose page shows its terminal. It reads the tasks alone.
import { memo } from "react";
import { taskLabel } from "../tasks.js";
import { Tile } from "../components/Icon.jsx";
import { useTasks } from "../hooks/useData.js";

const Running = memo(function Running() {
  const { data, error } = useTasks();
  if (!data) return error
    ? <p className="empty">{`Could not load this page: ${error.message}. Check that `}<code>priory serve</code> or the daemon runs, then retry.</p>
    : <p className="empty" role="status">Loading…</p>;
  const running = data.tasks.filter((task) => task.state === "running");
  if (!running.length) return <p className="empty">{"No agent runs now. A running task's terminal shows here."}</p>;
  return (
    <ol className="hist" aria-label="Running tasks">{running.map((task) => (
      <li key={task.id}><Tile state="running" size="sm" /><a href={`/tasks/${task.id}#terminal`} title={`task ${task.id}`}><span className="mono">{taskLabel(task.key)}</span>{` ${task.title || ""}`}</a></li>
    ))}</ol>
  );
});

/** The panel beside others. */
export const TerminalList = memo(function TerminalList() {
  return <div className="list" role="region" aria-label="Terminal"><div className="lhead"><h1>Terminal</h1></div><Running /></div>;
});

/** The panel alone on a phone. */
export const TerminalPage = memo(function TerminalPage() {
  return <><div className="top"><h1>Terminal</h1></div><Running /></>;
});
