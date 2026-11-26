// The Inbox panel (DESIGN.md, Inbox): the events no task heard or that reached no task, each group with
// what gets its next event to an agent. It reads `/api/inbox?unrouted=1` alone.
import { memo } from "react";
import { acts, writeTask } from "../acts.js";
import { refresh } from "../data.js";
import { droppedWhy, eventWords, neverOwned, requestedWorkflow, routeAction, unheardActs } from "../inbox.js";
import { post, refuse, session } from "../session.js";
import { store } from "../store.js";
import { taskLabel } from "../tasks.js";
import { PageTop, Panel } from "../components/PhonePage.jsx";
import { SavedViews } from "../components/SavedViews.jsx";
import { TaskActs } from "../components/TaskActs.jsx";
import { Tile } from "../components/Icon.jsx";
import { Time } from "../components/Text.jsx";
import { useInbox } from "../hooks/useInbox.js";
import { useStore } from "../hooks/useData.js";
import { addOpen, adding, changeAdd, keepDraft, openAdd, readAddDefaults } from "../components/AddSheet.jsx";
import { usePlace } from "./place.js";

// The resource whose subscribe is in flight, and each resource's last note. A phone's rotation swaps
// the List for the Page, so the notes live here, not in either.
export const subscribing = store({ busy: null, notices: new Map() });

// Subscribes a task to the resource, as `priory subscribe`, with the revision the page showed.
export async function subscribeTask(resource, task, label, revision) {
  subscribing.set({ ...subscribing.get(), busy: resource });
  let reply;
  try {
    // A group shown from an older read is refused.
    reply = await post("/api/subscribe", { task, resource, revision });
  } catch {
    reply = { status: 0, body: { error: "the connection dropped" } };
  }
  const { status, body } = reply;
  const notices = new Map(subscribing.get().notices);
  if (status === 401) {
    subscribing.set({ busy: null, notices });
    return refuse();
  }
  notices.set(resource, status === 200 ? `Subscribed: ${body.ok}. Its next event goes to ${label}.`
    : status === 409 ? "The state changed since this page loaded. Check the task, then subscribe again."
      : status === 503 ? "Could not send: no priory daemon runs beside this server. Start priory daemon, then send again."
        : status === 504 || status === 0 ? "The daemon may have applied this but did not confirm it. Check the task before you send again."
          : `Could not subscribe: ${body.error || `the server answered ${status}`}.`);
  subscribing.set({ busy: null, notices });
  refresh();
}

// The Add sheet with the server's text, whose URL keys the new task by the group's resource.
export async function requestTask(text, workflow) {
  await readAddDefaults();
  // The person may have opened the sheet and typed while the read ran.
  if (addOpen()) return;
  const draft = { ...adding.get().draft, text };
  const offered = requestedWorkflow(workflow, adding.get().defaults?.opted_out);
  if (offered === undefined) delete draft.workflow;
  else draft.workflow = offered;
  changeAdd({ draft, swapped: offered === undefined ? workflow : null });
  keepDraft();
  openAdd();
}

// A group's 5 newest events as history rows, then how many are older.
const EventRows = memo(function EventRows({ events, state: shown }) {
  return (
    <>
      <ol className="hist">{events.slice(0, 5).map((event, at) => (
        <li key={at}><Tile state={shown} size="sm" /><span><b title={event.type}>{eventWords(event.type)}</b>{` ${event.subject || ""}`}{event.line ? <span className="meta">{event.line}</span> : null}</span><Time iso={event.time} /></li>
      ))}</ol>
      {events.length > 5 ? <p className="meta">{`and ${events.length - 5} older`}</p> : null}
    </>
  );
});

const TaskLink = ({ task }) => <a className="mono" href={`/tasks/${task.id}`} title={`task ${task.id}`}>{taskLabel(task.key)}</a>;

// The one button that routes a group's next event: revive, request a task or subscribe.
const RouteButton = memo(function RouteButton({ group, action, revision }) {
  const { busy } = useStore(subscribing);
  const { task } = group;
  if (action === "revive") return <TaskActs task={task.id} state={task.state} revision={revision} />;
  if (action === "request") return group.request_text
    ? <button type="button" className="btn secondary" onClick={() => requestTask(group.request_text, task.workflow)}>Request a task</button>
    : <p className="meta">No new task can take this resource from here: start one from its thread.</p>;
  const off = busy === group.resource;
  return <button type="button" className="btn secondary" aria-disabled={off ? "true" : undefined} onClick={() => { if (subscribing.get().busy === null) subscribeTask(group.resource, task.id, taskLabel(task.key), revision); }}>{`Subscribe ${taskLabel(task.key)}`}</button>;
});

const OwnedGroup = memo(function OwnedGroup({ group, revision }) {
  const { notices } = useStore(subscribing);
  const { task } = group;
  const notice = notices.get(group.resource);
  return (
    <section className="ugroup" aria-label={group.resource}>
      <p className="l1"><span className="mono">{group.resource}</span><span className="num">{group.events.length}</span></p>
      <p className="meta"><Tile state={task.state} size="sm" /><TaskLink task={task} />{` ${task.state}`}</p>
      <EventRows events={group.events} state="queued" />
      <div className="routeform"><RouteButton group={group} action={routeAction(task)} revision={revision} /></div>
      {notice ? <p className="note" role="status">{notice}</p> : null}
    </section>
  );
});

// A request a pre script dropped (DESIGN.md, Inbox): its trigger, script, source event and why, with no action.
const DroppedGroup = memo(function DroppedGroup({ drop }) {
  const events = drop.event ? [drop.event] : [];
  return (
    <section className="ugroup" aria-label={drop.key || "dropped request"}>
      <p className="l1"><span className="mono">{drop.key || "-"}</span><span className="num">{events.length}</span></p>
      <p className="meta"><Tile state="dropped" size="sm" />{`trigger ${drop.trigger}, ${drop.script}`}</p>
      <EventRows events={events} state="dropped" />
      <p className="meta">{droppedWhy(drop)}</p>
    </section>
  );
});

// One task's unheard events (DESIGN.md, Inbox), with what gets them to an agent.
const UnheardGroup = memo(function UnheardGroup({ group, revision }) {
  const { busy } = useStore(acts);
  const { failed } = useStore(session);
  const { task } = group;
  const [first, ...rest] = unheardActs(task);
  const off = busy !== null || Boolean(failed);
  const act = first === "say"
    ? <>
        <button type="button" className="btn secondary" aria-disabled={off ? "true" : "false"} onClick={() => { if (!off) writeTask(task.id, "say", { text: group.say_text }); }}>{`Say to ${taskLabel(task.key)}`}</button>
        <TaskActs task={task.id} state={task.state} only={rest} revision={revision} />
        <p className="meta">{`Say sends “${group.say_text}”${rest.includes("resume") ? " It reads them when it resumes." : ""}`}</p>
      </>
    : first ? <RouteButton group={{ ...group, resource: task.key }} action={first} revision={revision} />
      : <p className="meta">Its next run reads them.</p>;
  return (
    <section className="ugroup" aria-label={taskLabel(task.key)}>
      <p className="l1"><TaskLink task={task} /><span className="num">{group.events.length}</span></p>
      <p className="meta"><Tile state={task.state} size="sm" />{`${task.state}, ${task.workflow}`}</p>
      <EventRows events={group.events} state="asleep" />
      <div className="routeform">{act}</div>
    </section>
  );
});

// The groups, or why there are none. A failed re-read keeps the last groups and says so above them.
const Rows = memo(function Rows({ data, error, retry }) {
  const failed = error ? <p className="empty">{`Could not read the unrouted events: ${error.message}. `}<button type="button" className="btn secondary" onClick={retry}>Try again</button></p> : null;
  if (!data) return failed ?? <p className="empty">Loading the unrouted events…</p>;
  const { owned, other, unheard = [], dropped = [] } = data.unrouted;
  const rest = neverOwned(other);
  const restLine = rest ? <p className="meta unowned">{rest}</p> : null;
  const heard = unheard.length || dropped.length
    ? <>
        {unheard.length ? <><div className="sec" id="unheard">Unheard<span>no agent read them</span></div>{unheard.map((group) => <UnheardGroup key={group.task.id} group={group} revision={data.revision} />)}</> : null}
        {dropped.length ? <><div className="sec" id="dropped">Dropped<span>a script judged each once</span></div>{dropped.map((drop, at) => <DroppedGroup key={at} drop={drop} />)}</> : null}
        <div className="sec">Unrouted<span>reached no task</span></div>
      </>
    : null;
  return (
    <>
      {failed}{heard}
      {owned.length ? owned.map((group) => <OwnedGroup key={group.resource} group={group} revision={data.revision} />)
        : <p className="empty">{"No event of a task's resource went unrouted. An event about an ended task's pull request or issue shows here."}</p>}
      {restLine}
    </>
  );
});

// Its one saved view, Unrouted, counts the events of a task's resources.
const Views = memo(function Views({ data }) {
  const count = data ? data.unrouted.owned.reduce((sum, group) => sum + group.events.length, 0) : null;
  return <SavedViews views={[{ key: "unrouted", label: "Unrouted", count, on: true }]} base="/inbox" />;
});

const KeyHints = () => <div className="keys"><span><kbd>J</kbd><kbd>K</kbd>move</span><span><kbd>Enter</kbd>open</span><span><kbd>Esc</kbd>close</span><span><kbd>N</kbd>add</span></div>;

/** The Inbox beside other panels. */
export const InboxList = memo(function InboxList() {
  const { data, error, refetch } = useInbox();
  const { size } = usePlace();
  return (
    <div className="list inbox" role="region" aria-label="Inbox">
      <div className="lhead"><h1>Inbox</h1></div>
      <Views data={data} />
      <div className="scroll"><Rows data={data} error={error} retry={() => refetch()} /></div>
      {size === "wide" ? <KeyHints /> : null}
    </div>
  );
});

/** The Inbox alone on a phone. */
export const InboxPage = memo(function InboxPage() {
  const { data, error, refetch } = useInbox();
  return (
    <>
      <PageTop title="Inbox" />
      <Panel label="Inbox"><div className="scroll"><Views data={data} /><Rows data={data} error={error} retry={() => refetch()} /></div></Panel>
    </>
  );
});
