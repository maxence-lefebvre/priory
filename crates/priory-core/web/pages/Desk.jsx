// The Desk (DESIGN.md, Desk): what needs a person, the one next thing, Up next and Since you looked,
// by saved view. A wide Desk draws a table; a phone's ends with the Answer capsule.
import { memo, useEffect, useRef } from "react";
import { askHeading, routedMark, runnerOf } from "../asks.js";
import { asked, cache, deskReadAt, refresh } from "../data.js";
import { actionNote, autopilotView, batchParts, capsuleWords, countHead, countWords, deskModel, dueParts, mergeSince, nextLink, onRoom, plural, rowAction, sinceGroups, taskRepo } from "../desk.js";
import { DESK_VIEWS } from "../frame.js";
import { role } from "../icons.js";
import { droppedNote, sinceWords, unheardNote, unroutedNote } from "../inbox.js";
import { post, refuse, session, writeRefusal } from "../session.js";
import { prStack } from "../stack.js";
import { store } from "../store.js";
import { taskLabel, workStep } from "../tasks.js";
import { age, ago, clock, date, today } from "../times.js";
import { openAdd } from "../components/AddSheet.jsx";
import { Connection } from "../components/Frame.jsx";
import { Icon, StateWord, Tile } from "../components/Icon.jsx";
import { NotReady } from "../components/NotReady.jsx";
import { SavedViews } from "../components/SavedViews.jsx";
import { TaskActs } from "../components/TaskActs.jsx";
import { Linked, Time } from "../components/Text.jsx";
import { useAsks, useDesk, useQueue, useStore, useTasks, useViews } from "../hooks/useData.js";
import { useKeptFocus } from "../hooks/useKeptFocus.js";
import { useKeptScroll } from "../hooks/useKeptScroll.js";
import { useOlder } from "../hooks/useOlder.js";
import { useMinute } from "../hooks/useScreen.js";
import { AskRow, AskTableRow, Fold, KeyHints, OlderRow, RunnerTag, SentLine, useListSearch } from "./Asks.jsx";
import { usePlace } from "./place.js";

// What the page holds beyond the reads: the row or to-do whose action is in flight, and each one's
// last refusal.
const acts = store({ rowBusy: null, rowNotices: new Map(), todoBusy: null, todoNotices: new Map() });
const set = (part) => acts.set({ ...acts.get(), ...part });

// Since you looked: the events the Desk opened on and each newer one, the visit they start from, when
// the first live read began, and whether POST /api/seen went.
const since = store({ events: [], from: null, opened: null, sent: false });
// The visit opens on the first live read of the desk. A desk kept on this device names no revision.
cache.getQueryCache().subscribe((event) => {
  if (event.type !== "updated" || event.action.type !== "success" || event.query.queryKey[0] !== "desk") return;
  const desk = event.query.state.data;
  if (desk?.revision === undefined) return;
  const held = since.get();
  const opened = held.opened ? {} : { opened: deskReadAt(), from: desk.last_visit };
  since.set({ ...held, ...opened, events: mergeSince(held.events, desk.since_you_looked) });
});

// Records the visit once, after Since you looked is drawn, at the time the first live read began.
async function seen() {
  const held = since.get();
  if (held.sent || !held.opened || session.get().failed) return;
  since.set({ ...held, sent: true });
  // As the Herdr Desk: one try. A visit not recorded shows the same events again next time.
  const { status, body } = await post("/api/seen", { at: held.opened }).catch((error) => ({ status: 0, body: { error: error.message } }));
  if (status !== 200) console.warn(`priory: could not record the visit: ${body.error || status}`);
}

/** The saved view of the Desk on screen. */
export const deskViewOf = (search) => {
  const name = new URLSearchParams(search).get("view");
  return DESK_VIEWS.includes(name) ? name : "needs";
};

// The Desk's four listings, and whether all of them are read.
function useDeskData() {
  const wanted = useStore(asked);
  const desk = useDesk();
  const asks = useAsks(wanted);
  const queue = useQueue();
  const tasks = useTasks(wanted);
  const results = [asks, queue, tasks];
  if (!desk.data || results.some((result) => !result.data)) return { results };
  const data = { desk: desk.data, asks: asks.data.asks, queue: queue.data, tasks: tasks.data.tasks };
  return { results, data, model: deskModel(data, ago), olderTasks: tasks.data.older };
}

// "3 PRs" for a task with a stack of two or more. The queue and desk carry no stacks; the tasks listing does.
const PrsTag = ({ task, id = task.id, tasks }) => {
  const { label } = prStack(task.stack ?? tasks.find((each) => each.id === id)?.stack);
  return label ? <span className="tag">{label}</span> : null;
};

// Read on a report and Release on a held task, the twins of `priory inbox read` and `priory release`.
async function rowAct(item, revision) {
  const action = rowAction(item, revision);
  const id = `${item.kind}-${item.id}`;
  set({ rowBusy: id });
  let reply;
  try {
    reply = await post(action.path, action.body);
  } catch {
    reply = { status: 0, body: { error: "the connection dropped" } };
  }
  const { status, body } = reply;
  if (status === 401) {
    set({ rowBusy: null });
    return refuse();
  }
  const rowNotices = new Map(acts.get().rowNotices);
  if (status === 200) rowNotices.delete(id);
  else rowNotices.set(id, actionNote(action.label, status, body));
  set({ rowBusy: null, rowNotices });
  refresh();
}

const RowActs = memo(function RowActs({ item, revision, prefix = "" }) {
  const { rowBusy, rowNotices } = useStore(acts);
  const { failed } = useStore(session);
  const action = rowAction(item, revision);
  const id = `${item.kind}-${item.id}`;
  // A cached desk has no revision yet: Release waits for the live read, or it would skip the 409 check.
  const busy = rowBusy !== null || Boolean(failed) || Object.values(action.body).includes(undefined);
  const notice = rowNotices.get(id);
  return (
    <>
      <span className="task-acts"><button type="button" id={`${prefix}row-${id}`} className="btn secondary act" aria-disabled={busy ? "true" : "false"} onClick={() => { if (!busy) rowAct(item, revision); }}>{action.label}</button></span>
      {notice ? <p className="note acts-note" role="status">{notice}</p> : null}
    </>
  );
});

async function markDone(id) {
  set({ todoBusy: id });
  let reply;
  try {
    reply = await post(`/api/todos/${id}/done`, {});
  } catch {
    reply = { status: 0, body: { error: "the connection dropped" } };
  }
  const { status, body } = reply;
  if (status === 401) {
    set({ todoBusy: null });
    return refuse();
  }
  const todoNotices = new Map(acts.get().todoNotices);
  if (status === 200) todoNotices.delete(id);
  else todoNotices.set(id, status === 503 ? "Could not send: no priory daemon runs beside this server. Start priory daemon, then send again."
    : status === 504 || status === 0 ? "The daemon may have applied this but did not confirm it. Check the desk before you send again."
      : `Could not mark it done: ${body.error || `the server answered ${status}`}.`);
  set({ todoBusy: null, todoNotices });
  refresh();
}

// `place` keeps the next thing card's button apart from the row's, so focus returns to the one tapped.
const TodoDone = memo(function TodoDone({ id, place = "row", prefix = "" }) {
  const { todoBusy, todoNotices } = useStore(acts);
  const { failed } = useStore(session);
  const busy = todoBusy !== null || Boolean(failed);
  const notice = todoNotices.get(id);
  return (
    <>
      <span className="task-acts"><button type="button" id={`${prefix}todo-${place}-${id}`} className="btn secondary act" aria-disabled={busy ? "true" : "false"} onClick={() => { if (!busy) markDone(id); }}>Done</button></span>
      {notice ? <p className="note acts-note" role="status">{notice}</p> : null}
    </>
  );
});

// Autopilot's off (DESIGN.md, Desk): the window it turned off, whether it is in flight, why no other
// runner was told, and why the last one did not go.
const pilot = store({ window: null, sending: false, untold: null, notice: null });

async function turnOff(window) {
  pilot.set({ ...pilot.get(), sending: true, notice: null });
  let reply;
  try {
    reply = await post("/api/autopilot/off", {});
  } catch {
    reply = { status: 0, body: {} };
  }
  const { status, body } = reply;
  if (status === 401) {
    pilot.set({ window: null, sending: false, untold: null, notice: null });
    return refuse();
  }
  pilot.set(status === 200 ? { window, sending: false, untold: body.untold ?? null, notice: null }
    : { ...pilot.get(), sending: false, notice: writeRefusal(status, body) });
  refresh();
}

const Autopilot = memo(function Autopilot({ autopilot }) {
  const off = useStore(pilot);
  const failed = Boolean(useStore(session).failed);
  const readAt = useDesk().dataUpdatedAt;
  const view = autopilotView(autopilot, off, failed && readAt ? new Date(readAt).toISOString() : null, { clock, ago });
  if (!view) return null;
  return (
    <div className="banner pilot" role="status"><Icon name="bot" size="s20" /><div><span className={view.error ? "t-danger" : undefined}>{view.words}</span><br />
      <button type="button" id="autopilot-off" className="btn secondary" aria-disabled={view.busy ? "true" : "false"} onClick={() => { if (!view.busy) turnOff(autopilot?.window ?? off.window); }}>{view.label}</button>
      {off.notice ? <p className="note acts-note" role="status">{off.notice}</p> : null}</div></div>
  );
});

// A to-do's text with its due time in local time; the `<time>` keeps the UTC time (DESIGN.md, Copy).
const TodoText = ({ item }) => {
  const due = dueParts(item.what, now());
  return due ? <><Linked text={`${due.text}, due `} /><Time iso={due.iso} text={due.label} /></> : <Linked text={item.what} />;
};

// The time a draw compares due times with; useMinute draws the Desk again each minute.
const now = () => Date.now();

const stoppedKind = (item) => ["blocked", "failed"].includes(item.kind);

// The actions of a waiting item, or null when it has none.
function itemActs(item, revision, prefix) {
  if (stoppedKind(item)) return <TaskActs task={item.task} state={item.kind} revision={revision} prefix={prefix} />;
  return rowAction(item, revision) ? <RowActs item={item} revision={revision} prefix={prefix} /> : null;
}

const ItemRow = memo(function ItemRow({ item, tasks, revision }) {
  if (item.kind === "memories") return <MemoriesRow item={item} />;
  // An uncertain command, like an uncertain prompt, waits on a person to check what happened.
  const kind = ["prompt", "command"].includes(item.kind) ? "prompt" : ["blocked", "failed", "held", "batch"].includes(item.kind) ? item.kind : "queued";
  const key = item.kind === "batch" ? <a className="mono" href={`/batches/${item.id}`}>{taskLabel(item.key)}</a>
    : stoppedKind(item) ? <a className="mono" href={`/tasks/${item.task}`} title={`task ${item.task}`}>{taskLabel(item.key)}</a> : <span className="mono">{taskLabel(item.key)}</span>;
  const { line, next } = item.kind === "batch" ? batchParts(item.what) : { line: item.what, next: null };
  const repo = taskRepo(tasks, item.task);
  return (
    <div className="row">
      <Tile state={kind} />
      <span className="l1">{key}<span>{item.step || ""}</span><span>{item.kind}</span>{item.task && item.kind !== "batch" ? <PrsTag task={{}} id={item.task} tasks={tasks} /> : null}<span className="age"><Time iso={item.since} /></span></span>
      <span className={`l2 r t-${role(kind)}`}><Linked text={line} repo={repo} /></span>{next ? <span className="l3"><Linked text={next} repo={repo} /></span> : null}{itemActs(item, revision, "")}</div>
  );
});

// The one row of every memory candidate that waits, which opens their review list.
const MemoriesRow = memo(function MemoriesRow({ item }) {
  const count = item.candidates?.length ?? 0;
  return (
    <a className="row" href="/memories">
      <Tile state="memories" />
      <span className="l1"><span>memories</span><span>{plural(count, "candidate")}</span><span className="age"><Time iso={item.since} /></span></span>
      <span className="l2 r">{(item.candidates || []).map((candidate) => candidate.name).join(", ")}</span>
      <span className="l3"><span>Accept or drop each before other tasks get it</span></span>
      <span className="chev"><Icon name="chevron-right" size="s15" /></span></a>
  );
});

const TodoRow = memo(function TodoRow({ item }) {
  return (
    <div className="row">
      <Tile state="todo" />
      <span className="l1"><span>to-do</span><span className="age"><Time iso={item.since} /></span></span>
      <span className="l2 r"><TodoText item={item} /></span><TodoDone id={item.id} /></div>
  );
});

// An asleep task with what each of its open waits waits on, and since when.
const AsleepRow = memo(function AsleepRow({ task, tasks }) {
  const waits = (task.waits || []).map((wait) => `${wait.kind} ${wait.on}, ${age(wait.since)}`).join("; ");
  return (
    <a className="row" href={`/tasks/${task.id}`}>
      <Tile state="asleep" />
      <span className="l1"><span className="mono">{taskLabel(task.key)}</span><span>{workStep(task)}</span><span>asleep</span><PrsTag task={task} tasks={tasks} /><span className="age"><Time iso={task.since} /></span></span>
      <span className="l2 r">{task.title || task.lane || ""}</span>
      <span className="l3"><span>{waits ? `Waits on ${waits}` : "Waits on an ask"}</span></span>
      <span className="chev"><Icon name="chevron-right" size="s15" /></span></a>
  );
});

const TaskRow = memo(function TaskRow({ task, tasks }) {
  return (
    <div className="row">
      <Tile state={task.state} />
      <span className="l1"><span className="mono">{taskLabel(task.key)}</span><span>{workStep(task)}</span><span>{task.state}</span><PrsTag task={task} tasks={tasks} /><span className="age"><Time iso={task.since} /></span></span>
      <span className={`l2 r ${task.reason ? `t-${role(task.state)}` : ""}`}><Linked text={task.reason || task.tldr || task.title || task.lane || ""} repo={task.repo} /></span></div>
  );
});

const RunRow = memo(function RunRow({ run }) {
  return (
    <div className="row">
      <Tile state="running" />
      <span className="l1"><span className="mono">{taskLabel(run.key)}</span><span>{run.step}</span><span>running</span><span className="age"><Time iso={run.started} /></span></span>
      <span className="l2 r">{`${run.agent}, run ${run.run}`}</span></div>
  );
});

const Empty = ({ children }) => <p className="empty">{children}</p>;

// Up next: the next three runs, and the queue that holds them.
const UpNext = memo(function UpNext({ upNext }) {
  const next = (upNext || []).slice(0, 3);
  const when = (run) => {
    if (run.kind === "queued") return onRoom(run) ? "now, when a runner has room" : "now";
    const time = clock(run.at);
    return today(run.at) ? time : `${time} ${new Date(run.at).toLocaleDateString("en", { weekday: "short" })}`;
  };
  const what = (run) => <><span className="mono">{run.key ? taskLabel(run.key) : run.trigger}</span>{` ${run.why}`}</>;
  const link = <a className="open" href="/queue">Open the queue</a>;
  return (
    <>
      <div className="sec">Up next<span>next 3 runs</span></div>
      {next.length
        ? <div className="strip"><ol>{next.map((run, at) => <li key={at}><Time iso={run.at} text={when(run)} />{run.task ? <a href={`/tasks/${run.task}`} title={`task ${run.task}`}>{what(run)}</a> : <span>{what(run)}</span>}</li>)}</ol>{link}</div>
        : <><p className="empty">Nothing is queued or scheduled to run next.</p>{link}</>}
    </>
  );
});

// Since you looked (DESIGN.md, Desk): what happened since the last visit, one group per task.
const SinceYouLooked = memo(function SinceYouLooked({ desk, tasks }) {
  const held = useStore(since);
  const failed = useStore(session).failed;
  // Until the first live read, the desk kept on this device: an offline start shows its events.
  const [events, from] = held.opened ? [held.events, held.from] : [desk.since_you_looked || [], desk.last_visit];
  const groups = sinceGroups(events);
  useEffect(() => {
    seen();
  }, [held.opened, failed]);
  return (
    <>
      <div className="sec">Since you looked<span>{from ? `since ${today(from) ? clock(from) : date(from)}` : "latest 50"}</span></div>
      {groups.length
        ? <div className="strip since" data-since>{groups.map((group, at) => (
            <div key={group.task ?? `f${at}`} className="sgroup">{group.task ? <a className="mono" href={`/tasks/${group.task}`} title={`task ${group.task}`}>{taskLabel(group.key)}</a> : <span className="mono">factory</span>}
              <ol>{group.events.map((event, n) => <li key={n}><Time iso={event.time} /><span><Linked text={sinceWords(event.what)} repo={taskRepo(tasks, event.task ?? group.task)} /></span></li>)}</ol></div>
          ))}</div>
        : <p className="empty" data-since>Nothing happened since you looked.</p>}
    </>
  );
});

// The fifth column's content, again under the question: CSS shows it when that column folds.
const review = <span className="task-acts"><a className="btn secondary act" href="/memories">Review</a></span>;

const DeskTable = memo(function DeskTable({ model, tasks, revision, selected }) {
  if (!model.open.length && !model.stopped.length && !model.todos.length && !model.other.length) return null;
  const itemLine = (item) => {
    if (item.kind === "memories") {
      return (
        <tr key={`${item.kind}-${item.id}`}>
          <td><StateWord state="memories" /></td><td className="mono">{""}</td><td>{""}</td><td className="q full"><span>{`${plural(item.candidates?.length ?? 0, "memory candidate")}: ${(item.candidates || []).map((candidate) => candidate.name).join(", ")}`}</span><Fold>{review}</Fold></td>
          <td className="recw">{review}</td><td className="r"><Time iso={item.since} /></td></tr>
      );
    }
    const repo = taskRepo(tasks, item.task);
    return (
      <tr key={`${item.kind}-${item.id}`}>
        <td><StateWord state={item.kind} /></td><td className="mono">{item.kind === "batch" ? <a href={`/batches/${item.id}`}>{taskLabel(item.key)}</a> : stoppedKind(item) ? <a href={`/tasks/${item.task}`} title={`task ${item.task}`}>{taskLabel(item.key)}</a> : taskLabel(item.key)}</td><td>{item.step || ""}</td>
        <td className={`q full t-${role(item.kind)}`}>{item.task ? <PrsTag task={{}} id={item.task} tasks={tasks} /> : null}<span><Linked text={item.what} repo={repo} /></span><Fold>{itemActs(item, revision, "fold-")}</Fold></td><td className="recw">{itemActs(item, revision, "")}</td><td className="r"><Time iso={item.since} /></td></tr>
    );
  };
  const todoLine = (item) => (
    <tr key={`todo-${item.id}`}>
      <td><StateWord state="todo" /></td><td className="mono">{""}</td><td>{""}</td><td className="q full"><span><TodoText item={item} /></span><Fold><TodoDone id={item.id} prefix="fold-" /></Fold></td><td className="recw"><TodoDone id={item.id} /></td><td className="r"><Time iso={item.since} /></td></tr>
  );
  const group = (title, count) => <tr key={title} className="grp"><td colSpan="6">{title}<span>{count}</span></td></tr>;
  return (
    <table className="tbl">
      <thead><tr><th>State</th><th>Task</th><th>Step</th><th>Waits on you</th><th>Recommends</th><th className="r">Age</th></tr></thead>
      <tbody>{model.open.map((ask) => <AskTableRow key={ask.id} ask={ask} selected={selected} />)}
        {model.stopped.length ? [group("Stopped", model.stopped.length), ...model.stopped.map(itemLine)] : null}
        {model.todos.length ? [group("To-dos", model.todos.length), ...model.todos.map(todoLine)] : null}
        {model.other.length ? [group("Also waiting", model.other.length), ...model.other.map(itemLine)] : null}
      </tbody></table>
  );
});

const CountRow = ({ href, state, text, n }) => <a className="row one calm" href={href}><Tile state={state} /><span>{text}</span><span className="num">{n}</span><span className="chev"><Icon name="chevron-right" size="s15" /></span></a>;

function NeedsMe({ model, data, wide, selected }) {
  const { desk, tasks } = data;
  const counts = desk.counts ?? {};
  const unrouted = unroutedNote(counts.unrouted);
  const unheard = unheardNote(counts.unheard);
  const dropped = droppedNote(counts.dropped);
  return (
    <>
      {!model.open.length ? <Empty>No open asks. Agents ask here when they need you.</Empty> : null}
      {wide ? <DeskTable model={model} tasks={tasks} revision={desk.revision} selected={selected} /> : (
        <>
          {model.open.length ? <><div className="sec">Asks<span>oldest first</span></div>{model.open.map((ask) => <AskRow key={ask.id} ask={ask} current={String(ask.id) === selected} />)}</> : null}
          {model.stopped.length ? <><div className="sec">Stopped<span>{model.stopped.length}</span></div>{model.stopped.map((item) => <ItemRow key={`${item.kind}-${item.id}`} item={item} tasks={tasks} revision={desk.revision} />)}</> : null}
          {model.todos.length ? <><div className="sec">To-dos<span>{model.todos.length}</span></div>{model.todos.map((item) => <TodoRow key={item.id} item={item} />)}</> : null}
          {model.other.length ? <><div className="sec">Also waiting<span>{model.other.length}</span></div>{model.other.map((item) => <ItemRow key={`${item.kind}-${item.id}`} item={item} tasks={tasks} revision={desk.revision} />)}</> : null}
        </>
      )}
      <UpNext upNext={model.upNext} /><SinceYouLooked desk={desk} tasks={tasks} />
      {model.asleep ? <><div className="sec" /><a className="row one" href="/?view=asleep"><Tile state="asleep" /><span>{`${plural(model.asleep, "task")} asleep on waits`}</span><span className="num">{model.asleep}</span><span className="chev"><Icon name="chevron-right" size="s15" /></span></a></> : null}
      {unrouted ? <CountRow href="/inbox?view=unrouted" state="queued" text={unrouted} n={counts.unrouted} /> : null}
      {unheard ? <CountRow href="/inbox?view=unrouted#unheard" state="asleep" text={unheard} n={counts.unheard} /> : null}
      {dropped ? <CountRow href="/inbox?view=unrouted#dropped" state="dropped" text={dropped} n={counts.dropped} /> : null}
      {model.reports.length ? <><div className="sec">Reports<span>{model.reports.length}</span></div>{model.reports.map((item) => <ItemRow key={`${item.kind}-${item.id}`} item={item} tasks={tasks} revision={desk.revision} />)}</> : null}
    </>
  );
}

// The Desk's parts above and in its scroll: the views, the banners and the rows of the view on screen.
function useDeskBody({ model, data, olderTasks }, wide) {
  useMinute();
  const search = useListSearch();
  const { at } = usePlace();
  const current = deskViewOf(search);
  useOlder("tasks", current === "all");
  useOlder("asks", Boolean(at.ask));
  const views = (
    <SavedViews base="/" views={[
      { key: "needs", label: "Needs me", count: model.needs, on: current === "needs" },
      { key: "running", label: "Running", count: model.running.length, on: current === "running" },
      ...(wide ? [{ key: "queue", label: "Queue", count: model.queue.length, on: current === "queue" }] : []),
      { key: "asleep", label: "Asleep", count: model.asleep, on: current === "asleep" },
      { key: "all", label: "All", count: wide ? model.tasks.length + olderTasks : null, on: current === "all" },
    ]} />
  );
  const banner = [<Autopilot key="autopilot" autopilot={data.desk.autopilot} />, ...model.banners.map(({ text, icon, intake }) => <div key={text} className="banner" role="status" id={intake ? "intake" : undefined}><Icon name={icon} size="s20" /><span>{text}</span></div>)];
  const { tasks } = data;
  let rows;
  if (current === "running") rows = model.running.length ? model.running.map((run) => <RunRow key={run.run} run={run} />) : <Empty>No run is live. Runs show here while an agent works.</Empty>;
  else if (current === "queue") rows = model.queue.length ? model.queue.map((task) => <TaskRow key={task.id} task={task} tasks={tasks} />) : <Empty>The queue is empty. A task waits here for its turn; see <code>priory queue</code>.</Empty>;
  else if (current === "asleep") rows = model.asleepTasks.length ? model.asleepTasks.map((task) => <AsleepRow key={task.id} task={task} tasks={tasks} />) : <Empty>No task is asleep. A task sleeps here while it waits on an event.</Empty>;
  else if (current === "all") rows = model.tasks.length || olderTasks ? [...model.tasks.map((task) => <TaskRow key={task.id} task={task} tasks={tasks} />), <OlderRow key="older" listing="tasks" count={olderTasks} wide={false} />] : <Empty>No tasks yet. Add one with the + button, or run <code>priory add</code>.</Empty>;
  else rows = <NeedsMe model={model} data={data} wide={wide} selected={at.ask} />;
  return { views, banner, rows, current, search };
}

const CountWords = ({ model }) => countWords({ oldest: model.open[0] ? age(model.open[0].since) : null, stopped: model.stopped.length, todos: model.todos.length, reports: model.reports.length, other: model.other.length });

// The one next thing (DESIGN.md, Desk): the desk's `next`, an alarm or the ask that ranks first, and why.
const NextCard = memo(function NextCard({ model, data }) {
  const item = model.next;
  if (!item) return null;
  const ask = item.kind === "ask" ? data.asks.find((each) => String(each.id) === String(item.id)) : null;
  const task = data.tasks.find((each) => each.id === item.task);
  const text = ask ? askHeading(ask) : ["blocked", "failed"].includes(item.kind) && task?.reason ? task.reason : item.what;
  const kind = ["prompt", "command"].includes(item.kind) ? "prompt" : item.kind === "report" ? "lost" : item.kind;
  const href = nextLink(item);
  const verb = ask ? "Answer…" : href?.startsWith("#") ? "See the banner" : "Open task";
  const repo = ask?.repo || task?.repo;
  return (
    <section className="next" aria-label="The next thing"><p className="l1"><Tile state={kind} size="sm" /><span className="mono">{taskLabel(item.key)}</span><span>{item.step || ""}</span><span className="age"><Time iso={item.since} /></span></p>
      <p className="q">{item.kind === "todo" ? <TodoText item={item} /> : <Linked text={text} repo={repo} />}</p>{ask && routedMark(ask) ? <p className="why t-live">{routedMark(ask)}</p> : null}{runnerOf(ask) ? <p className="why"><RunnerTag ask={ask} />{" "}<SentLine ask={ask} /></p> : null}<p className={`why t-${role(kind)}`}><Linked text={item.why} repo={repo} /></p>
      {ask?.recommendation ? <p className="rec"><Icon name="corner-down-right" size="s15" /><span>{ask.recommendation}</span></p> : null}
      {item.kind === "todo" ? <TodoDone id={item.id} place="next" /> : href ? <a className={`btn ${ask ? "primary" : "secondary"}`} href={href}>{verb}</a> : null}</section>
  );
});

// On a phone the Add button sits beside the capsule, or alone where the capsule hides.
const Capsule = memo(function Capsule({ model, current }) {
  const n = model.open.length;
  const words = capsuleWords(n, Boolean(useStore(session).failed));
  return (
    <><div className="fade" /><div className="dock">{n && current === "needs" ? <a className={`capsule ${words.off ? "off" : ""}`} href={`/asks/${model.first.id}`}><Icon name={words.icon} size="s20" />{words.text}</a> : null}
      <button type="button" className="fab" data-action="add-open" aria-label="Add a task" aria-keyshortcuts="N" aria-haspopup="dialog" onClick={openAdd}><Icon name="plus" size="s20" /></button></div></>
  );
});

const Head = ({ model }) => {
  const head = countHead(model.open.length);
  return <>{head.n ? <span className="n num">{head.n}</span> : null}<span className="w">{head.words}<span><CountWords model={model} /></span></span></>;
};

// The Desk on a phone, and in each theme's preview, which has no capsule.
const DeskPhone = memo(function DeskPhone({ read, preview }) {
  const factory = useViews().data?.factory;
  const { views, banner, rows, current, search } = useDeskBody(read, false);
  const scroll = useKeptScroll(`desk phone ${preview} ${search}`);
  const { model } = read;
  return (
    <>
      <div className="top"><span className="fac">{factory}</span><Connection /></div>
      <div className="panel" role="region" aria-label="Desk"><h1 className="sr">Desk</h1><div className={`scroll ${preview ? "" : "desk-end"}`} ref={scroll}>
        <div className={`count ${countHead(model.open.length).n ? "" : "quiet"}`}><Head model={model} /></div>
        <NextCard model={model} data={read.data} />{views}{banner}{rows}</div>{preview ? null : <Capsule model={model} current={current} />}</div>
    </>
  );
});

/** The Desk beside other panels. */
export const DeskList = memo(function DeskList() {
  const read = useDeskData();
  return read.data ? <DeskListBody read={read} /> : <NotReady results={read.results} />;
});

const DeskListBody = memo(function DeskListBody({ read }) {
  const wide = usePlace().size === "wide";
  const body = useDeskBody(read, wide);
  const scroll = useKeptScroll(`desk list ${body.search}`);
  const part = useRef(null);
  useKeptFocus(part);
  const { model } = read;
  return (
    <div className="list" role="region" aria-label="Desk" ref={part}><div className="lhead"><h1>Desk</h1>
      <span className="cnt"><Head model={model} /></span></div>
      <NextCard model={model} data={read.data} />{body.views}{body.banner}<div className="scroll" ref={scroll}>{body.rows}</div>{wide ? <KeyHints /> : null}</div>
  );
});

/** The Desk alone on a phone. */
export const DeskPage = memo(function DeskPage() {
  const read = useDeskData();
  return <div className="flat">{read.data ? <DeskPhone read={read} preview={false} /> : <NotReady results={read.results} />}</div>;
});

/** The Desk drawn small under a theme in Settings, once its listings are read. */
export const DeskPreview = memo(function DeskPreview() {
  const read = useDeskData();
  return <div>{read.data ? <DeskPhone read={read} preview /> : null}</div>;
});

