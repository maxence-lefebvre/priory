// Queue (DESIGN.md, Queue panel): the waiting tasks by band in admission order, why each waits, and a
// Move menu that sends `priory move` at the revision the page drew.
import { Fragment, memo } from "react";
import { refresh } from "../data.js";
import { actionNote, onRoom } from "../desk.js";
import { QUEUE_VIEWS, laneCap, moveOptions, queueBands, waitReason } from "../queue.js";
import { post, refuse } from "../session.js";
import { store } from "../store.js";
import { taskLabel } from "../tasks.js";
import { clock, today } from "../times.js";
import { Icon, StateWord, Tile } from "../components/Icon.jsx";
import { PageLoading, RowsList, RowsPage, listView } from "../components/Lists.jsx";
import { SavedViews } from "../components/SavedViews.jsx";
import { TaskListRow } from "../components/TaskRows.jsx";
import { Time } from "../components/Text.jsx";
import { useQueue, useStore, useTasks } from "../hooks/useData.js";
import { usePlace } from "./place.js";

// The task whose Move is in flight, each task's last Move refusal, and the task whose Move menu is
// open. A rotation swaps the List for the Page, so they live here.
export const moving = store({ busy: null, notices: new Map(), open: null });
const change = (part) => moving.set({ ...moving.get(), ...part });

/** Sends one Move option of a task, as `priory move --expected`; a refusal shows under the row. */
export async function moveTask(task, option) {
  if (moving.get().busy !== null) return;
  change({ busy: task.id });
  let reply;
  try {
    reply = await post(`/api/tasks/${task.id}/move`, option.body);
  } catch {
    reply = { status: 0, body: { error: "the connection dropped" } };
  }
  const { status, body } = reply;
  const notices = new Map(moving.get().notices);
  if (status === 401) {
    change({ busy: null, open: null });
    return refuse();
  }
  if (status === 200) notices.delete(task.id);
  else notices.set(task.id, actionNote("Move", status, body));
  change({ busy: null, open: null, notices });
  document.getElementById(`move-${task.id}`)?.focus();
  refresh();
}

/** Esc closes an open Move menu and gives its button the focus. */
export function queueKey(event) {
  const menu = event.key === "Escape" && document.querySelector("details.move[open]");
  if (!menu) return false;
  change({ open: null });
  menu.querySelector("summary").focus();
  return true;
}

const MoveMenu = memo(function MoveMenu({ task, bands, queue }) {
  const { busy, open } = useStore(moving);
  const options = moveOptions(task, bands, queue.bands, queue.revision);
  const toggle = (event) => {
    if (event.currentTarget.open) change({ open: task.id });
    else if (moving.get().open === task.id) change({ open: null });
  };
  return (
    <details className="move" data-move={task.id} open={open === task.id} onToggle={toggle}><summary id={`move-${task.id}`} className="btn secondary" aria-label={`Move ${taskLabel(task.key)}`}>Move<Icon name="chevron-down" size="s13" /></summary>
      <div className="pick">{options.map((option, at) => (
        <button key={option.label} type="button" id={`move-${task.id}-${at}`} data-action="move" data-task={task.id} data-option={at} aria-disabled={busy === task.id ? "true" : undefined} onClick={() => moveTask(task, option)}>{option.label}</button>
      ))}</div></details>
  );
});

const MoveNote = memo(function MoveNote({ task }) {
  const notice = useStore(moving).notices.get(task.id);
  return notice ? <p className="note warn" role="status"><Icon name="circle-x" size="s15" /><span>{notice}</span></p> : null;
});

// A table's cell is the mono one, a list's link.
const KeyLink = ({ task, cell }) => <a className={cell ? undefined : "mono"} href={`/tasks/${task.id}`} title={`task ${task.id}`}>{taskLabel(task.key)}</a>;

const QueueListRow = memo(function QueueListRow({ task, bands, queue }) {
  return (
    <div className="row qrow" data-href={`/tasks/${task.id}`}><Tile state={task.state} />
      <span className="l1"><span className="num">{`#${task.band_position}`}</span><KeyLink task={task} /><span>{task.state}</span><span className="age"><Time iso={task.since} /></span></span>
      <span className="l2 r">{task.title || ""}</span><span className="l3 wrap">{waitReason(task, bands)}</span><MoveMenu task={task} bands={bands} queue={queue} /><MoveNote task={task} /></div>
  );
});

const QueueTableRow = memo(function QueueTableRow({ task, bands, queue }) {
  return (
    <tr data-href={`/tasks/${task.id}`}><td className="r">{`#${task.band_position}`}</td><td><StateWord state={task.state} /></td><td className="mono"><KeyLink task={task} cell /></td>
      <td className="q full"><span>{task.title || ""}</span><span className="why t3">{waitReason(task, bands)}</span><MoveNote task={task} /></td><td className="act"><MoveMenu task={task} bands={bands} queue={queue} /></td></tr>
  );
});

// The next 3 runs, queued or scheduled, as the Desk's Up next draws them, without its queue link.
const UpNext = memo(function UpNext({ runs = [] }) {
  const next = runs.slice(0, 3);
  const when = (run) => {
    if (run.kind === "queued") return onRoom(run) ? "now, when a runner has room" : "now";
    const time = clock(run.at);
    return today(run.at) ? time : `${time} ${new Date(run.at).toLocaleDateString("en", { weekday: "short" })}`;
  };
  const what = (run) => <><span className="mono">{run.key ? taskLabel(run.key) : run.trigger}</span>{` ${run.why}`}</>;
  return (
    <>
      <div className="sec">Up next<span>next 3 runs</span></div>
      {next.length
        ? <div className="strip"><ol>{next.map((run, at) => <li key={at}><Time iso={run.at} text={when(run)} />{run.task ? <a href={`/tasks/${run.task}`} title={`task ${run.task}`}>{what(run)}</a> : <span>{what(run)}</span>}</li>)}</ol></div>
        : <p className="empty">Nothing is queued or scheduled to run next.</p>}
    </>
  );
});

// A daemon from before lane caps sends no `lanes`.
const LaneCaps = memo(function LaneCaps({ lanes = [] }) {
  if (!lanes.length) return null;
  return (
    <>
      <div className="sec">Lane caps<span>runs per lane on this runner</span></div>
      <ul className="strip caps">{lanes.map((each) => {
        const { lane, now, cap, full } = laneCap(each);
        return <li key={lane}><span className="mono">{lane}</span><span className={full ? "t-warn" : ""}>{now}</span><span className="cap">{cap}</span></li>;
      })}</ul>
    </>
  );
});

const Empty = ({ rows, text }) => (rows.length ? rows : <p className="empty">{text}</p>);

// The Held and Last day views draw task rows, which name a task's pull requests from the tasks listing.
const TaskRows = memo(function TaskRows({ tasks, empty }) {
  const stacks = new Map((useTasks().data?.tasks ?? []).map((task) => [task.id, task.stack]));
  return <Empty rows={tasks.map((task) => <TaskListRow key={task.id} task={task} stack={task.stack ?? stacks.get(task.id)} />)} text={empty} />;
});

function body(queue, current, wide) {
  const bands = queueBands(queue);
  const held = queue.waiting.filter((task) => task.state === "held");
  const waiting = bands.reduce((sum, group) => sum + group.tasks.length, 0);
  const views = <SavedViews views={[
    { key: "waiting", label: "Waiting", count: waiting, on: current === "waiting" },
    { key: "held", label: "Held", count: held.length, on: current === "held" },
    { key: "ended", label: "Last day", count: queue.ended.length, on: current === "ended" },
  ]} base="/queue" />;
  if (current === "held") return { views, rows: <TaskRows tasks={held.map((task) => ({ ...task, reason: task.reason || "held" }))} empty="No task is held. A task waits here while a person or a condition holds it." /> };
  if (current === "ended") return { views, rows: <TaskRows tasks={queue.ended.map((task) => ({ ...task, since: task.ended }))} empty="No task ended in the last day." /> };
  const strip = <><UpNext runs={queue.up_next} /><LaneCaps lanes={queue.lanes} /></>;
  if (!bands.length) return { views, rows: <>{strip}<p className="empty">The queue is empty. A task waits here for its turn once it is added.</p></> };
  if (wide) {
    // oxlint-disable-next-line jsx-a11y/control-has-associated-label -- the Move column has no title, as before React
    return { views, rows: <>{strip}<table className="tbl q"><thead><tr><th className="r">#</th><th>State</th><th>Task</th><th>Title and why it waits</th><th /></tr></thead><tbody>{bands.map((group) => (
      <Fragment key={group.band}><tr className="grp"><td colSpan={5}>{`Band ${group.band}`}<span>{group.tasks.length}</span></td></tr>{group.tasks.map((task) => <QueueTableRow key={task.id} task={task} bands={bands} queue={queue} />)}</Fragment>
    ))}</tbody></table></> };
  }
  return { views, rows: <>{strip}{bands.map((group) => (
    <Fragment key={group.band}><div className="sec">{`Band ${group.band}`}<span>{group.tasks.length}</span></div>{group.tasks.map((task) => <QueueListRow key={task.id} task={task} bands={bands} queue={queue} />)}</Fragment>
  ))}</> };
}

function useQueuePage() {
  const { href } = usePlace();
  return { ...useQueue(), href, current: listView(href, QUEUE_VIEWS) };
}

/** The Queue beside other panels. */
export const QueueList = memo(function QueueList() {
  const { data, error, href, current } = useQueuePage();
  const { size } = usePlace();
  if (!data) return <PageLoading error={error} />;
  const { views, rows } = body(data, current, size === "wide");
  return <RowsList label="Queue" href={href} views={views}>{rows}</RowsList>;
});

/** The Queue alone on a phone. */
export const QueuePage = memo(function QueuePage() {
  const { data, error, href, current } = useQueuePage();
  if (!data) return <PageLoading error={error} />;
  const { views, rows } = body(data, current, false);
  return <RowsPage label="Queue" href={href} views={views}>{rows}</RowsPage>;
});
