// A task's row in a list or a table (DESIGN.md, Task row and run row), as Tasks and Queue draw it.
// Another runner's task has its hold for a state, and no page here: a tap opens its actions in place.
import { memo, useEffect } from "react";
import { acts, disarmTask, draft, writeTask } from "../acts.js";
import { role } from "../icons.js";
import { prStack } from "../stack.js";
import { store } from "../store.js";
import { elsewhere, remoteActs, remoteId, remoteSent, taskLabel, workStep } from "../tasks.js";
import { ago } from "../times.js";
import { useStore } from "../hooks/useData.js";
import { Icon, StateWord, Tile } from "./Icon.jsx";
import { TaskActs } from "./TaskActs.jsx";
import { Time } from "./Text.jsx";

// The remote task row opened in place, by its id `runner:key`. A rotation swaps the List for the Page.
export const remoteOpen = store(null);
const toggleRemote = (id) => remoteOpen.set(remoteOpen.get() === id ? null : id);

export const reasonShown = (task) => Boolean(task.reason) && ["blocked", "held", "failed", "dropped"].includes(task.state);

const BatchTag = ({ task }) => (task.batch == null ? null : <a className="tag" href={`/batches/${task.batch}`}><Icon name="layers" size="s13" />{`batch ${task.batch}`}</a>);
// "3 PRs" for a task with a stack of two or more. The queue carries no stacks; the tasks listing does.
const PrsTag = ({ stack }) => {
  const { label } = prStack(stack);
  return label ? <span className="tag">{label}</span> : null;
};
// A table's cell is the mono one, a list's link.
const KeyLink = ({ id, label, cell }) => <a className={cell ? undefined : "mono"} href={`/tasks/${id}`} title={`task ${id}`}>{label}</a>;

const TONE = { action: "t-action", warn: "t-warn", quiet: "" };
const Sent = ({ task }) => {
  const sent = remoteSent(task);
  return sent ? <span className={TONE[sent.tone]}>{sent.text}</span> : null;
};

const Toggle = ({ id, open, label }) => (
  <button type="button" id={`remote-${id}`} className="chev" data-action="remote-open" data-task={id} aria-expanded={open ? "true" : "false"} aria-label={`${open ? "Close" : "Open"} the actions of ${label}`}>
    <Icon name={open ? "chevron-up" : "chevron-down"} size="s15" />
  </button>
);

const RemoteSay = memo(function RemoteSay({ id, generation }) {
  const { busy, drafts } = useStore(acts);
  const text = drafts.get(id) ?? "";
  const submit = (event) => {
    event.preventDefault();
    if (text.trim() && busy === null) writeTask(id, "say", { text: text.trim(), generation });
  };
  return (
    <form className="writeform" onSubmit={submit}>
      <label className="sr" htmlFor="say">Say to the agent</label>
      <textarea id="say" name="text" rows="2" placeholder="Say to the agent" value={text} onChange={(event) => draft(id, event.target.value)} />
      <button type="submit" className="btn primary" aria-disabled={busy === id ? "true" : undefined}>Say</button>
    </form>
  );
});

function RemoteActions({ task }) {
  const id = remoteId(task);
  const armed = useStore(acts).armed;
  // A Cancel armed on another runner's task holds for the generation the list shows.
  useEffect(() => {
    if (armed?.task === id && armed.generation !== task.generation) disarmTask();
  }, [armed, id, task.generation]);
  return (
    <>
      <TaskActs task={id} only={remoteActs(task)} generation={task.generation} />
      {task.hold === "kept" ? <RemoteSay id={id} generation={task.generation} /> : null}
      <p className="note acts-note"><Icon name="corner-down-right" size="s15" /><span>{`This task runs on ${task.runner}. An action goes there through the log, and ${task.runner} applies it when it reads it.`}</span></p>
    </>
  );
}

/** A tap on a remote row opens its actions, but not one inside its actions or its Say. True: it was one. */
export function remoteClick(event) {
  const row = event.target.closest('[data-action="remote-open"]');
  if (!row || event.target.closest("form, .task-acts")) return Boolean(row);
  toggleRemote(row.dataset.task);
  return true;
}

const RemoteListRow = memo(function RemoteListRow({ task }) {
  const id = remoteId(task);
  const open = useStore(remoteOpen) === id;
  const label = taskLabel(task.key);
  const sent = remoteSent(task);
  return (
    <div className="row remote" data-action="remote-open" data-task={id}><Tile state="elsewhere" />
      <span className="l1"><span className="mono">{label}</span><span>{task.workflow || ""}</span><span>{task.hold}</span><span className="age"><Time iso={task.since} /></span></span>
      <span className="l2 r">{task.title || label}</span>{sent ? <span className="l3"><Sent task={task} /></span> : null}<span className="l3"><span>{elsewhere(task, ago)}</span></span>
      <Toggle id={id} open={open} label={label} />{open ? <RemoteActions task={task} /> : null}</div>
  );
});

const RemoteTableRow = memo(function RemoteTableRow({ task }) {
  const id = remoteId(task);
  const open = useStore(remoteOpen) === id;
  const label = taskLabel(task.key);
  const sent = remoteSent(task);
  return (
    <>
      <tr className="remote" data-action="remote-open" data-task={id}><td><span className="sti"><Tile state="elsewhere" size="sm" />{task.hold}</span></td><td className="mono">{label}</td>
        <td className="q full"><span>{task.title || label}</span>{sent ? <span className="why"><Sent task={task} /></span> : null}<span className="why t3">{elsewhere(task, ago)}</span></td>
        <td>{task.workflow || ""}</td><td>{task.lane || ""}</td><td>{task.band || ""}</td><td className="r"><Time iso={task.since} />{" "}<Toggle id={id} open={open} label={label} /></td></tr>
      {/* oxlint-disable-next-line jsx-a11y/control-has-associated-label -- the empty cells line the actions up under the title, as before React */}
      {open ? <tr className="remote-acts"><td /><td /><td colSpan={5}><RemoteActions task={task} /></td></tr> : null}
    </>
  );
});

/** A task's row in a list. `stack` is its pull requests, from the tasks listing. */
export const TaskListRow = memo(function TaskListRow({ task, stack = task.stack }) {
  if (elsewhere(task, ago)) return <RemoteListRow task={task} />;
  const line3 = reasonShown(task) ? <><span className={`t-${role(task.state)}`}>{task.reason}</span><BatchTag task={task} /></> : <><span>{task.lane || ""}</span><span>{task.band || ""}</span><BatchTag task={task} /></>;
  return (
    <div className="row" data-href={`/tasks/${task.id}`}><Tile state={task.state} />
      <span className="l1"><KeyLink id={task.id} label={taskLabel(task.key)} /><span>{workStep(task)}</span><span>{task.state}</span><PrsTag stack={stack} /><span className="age"><Time iso={task.since} /></span></span>
      <span className="l2 r">{task.title || task.tldr || ""}</span><span className="l3">{line3}</span><span className="chev"><Icon name="chevron-right" size="s15" /></span></div>
  );
});

/** A task's row in a table. */
export const TaskTableRow = memo(function TaskTableRow({ task }) {
  if (elsewhere(task, ago)) return <RemoteTableRow task={task} />;
  return (
    <tr data-href={`/tasks/${task.id}`}><td><StateWord state={task.state} /></td><td className="mono"><KeyLink id={task.id} label={taskLabel(task.key)} cell /></td>
      <td className="q full"><span>{`${task.title || task.tldr || ""}${task.batch == null ? "" : " "}`}<BatchTag task={task} /></span>{reasonShown(task) ? <span className={`why t-${role(task.state)}`}>{task.reason}</span> : null}</td>
      <td>{`${task.workflow ?? ""} `}<span className="t3">{task.step}</span></td><td>{task.lane || ""}</td><td>{task.band || ""}</td><td className="r"><Time iso={task.since} /></td></tr>
  );
});
