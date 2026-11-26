// The task page (DESIGN.md, Task detail): state, reason, asks, actions, a box to say something to its
// agent, then its brief, plan, pull request, trace and terminal.
import { memo, useEffect, useLayoutEffect } from "react";
import { ENDED, acts, draft, writeTask } from "../acts.js";
import { answeredAsks, askHeading, confidenceNote, courseNote, openAsksOf } from "../asks.js";
import { Icon, StateWord, Tile } from "../components/Icon.jsx";
import { TaskActs } from "../components/TaskActs.jsx";
import { Section, TaskSections } from "../components/TaskSections.jsx";
import { Linked, Time } from "../components/Text.jsx";
import { useAsks, useStore, useTask, useTasks } from "../hooks/useData.js";
import { TerminalPane, terminalRun } from "../components/TerminalPane.jsx";
import { role } from "../icons.js";
import { backTo } from "../pages.js";
import { toHash } from "../router.js";
import { taskLabel, workStep } from "../tasks.js";
import { listClass, usePlace } from "./place.js";

// The task's open asks, each with a way to answer it.
const OpenAsks = memo(function OpenAsks({ asks }) {
  if (!asks.length) return null;
  return (
    <ul className="taskasks" aria-label="Open asks">{asks.map((ask) => (
      <li key={ask.id}><Tile state="ask" size="sm" /><span>{askHeading(ask)}<span className="age"><Time iso={ask.since} /></span></span><a className="btn primary" href={`/asks/${encodeURIComponent(ask.id)}`}>Answer…</a></li>
    ))}</ul>
  );
});

// Who answered, as the answered row's icon and what a screen reader reads of it.
const ANSWERERS = { auto: ["cog", "Answered by auto"], autopilot: ["cog", "Answered by autopilot"], agent: ["cog", "Answered by an agent"], session: ["terminal", "Answered from a session"], person: ["user", "Answered by a person"] };

const Confidence = memo(function Confidence({ level, bars }) {
  return <span className="bars" role="img" aria-label={`Confidence ${level}`}>{[1, 2, 3].map((n) => <i key={n} className={n <= bars ? "on" : undefined} />)}</span>;
});

const By = memo(function By({ ask }) {
  if (!ask.by) return null;
  if (ask.answerer === "session") return <> by session <span className="mono">{ask.by.slice(8)}</span></>;
  return ask.answerer === "person" ? ` by ${ask.by}` : <> by <b>{ask.by}</b></>;
});

// An auto answer opens to its confidence, rationale and trade-off, and says how to change course.
const Answered = memo(function Answered({ ask, live, level }) {
  const [name, label] = ANSWERERS[ask.answerer] || ["check", "Answered"];
  const alone = ask.answerer === "auto" || ask.answerer === "autopilot";
  const head = <><p className="aq">{ask.title}</p><p className="said">Answered <b>{ask.answer}</b><By ask={ask} />{ask.recommended == null ? "" : `. Recommended ${ask.recommended}.`}</p></>;
  if (!alone) {
    return (
      <li><Icon name={name} size="who" label={label} />
        <div>{head}{ask.confidence ? <p className="sure dim"><Confidence level={ask.confidence} bars={ask.bars} />{`${ask.confidence}${ask.rationale ? `: ${ask.rationale}` : ""}`}</p> : null}</div><Time iso={ask.at} /></li>
    );
  }
  const why = [["Why", ask.rationale], ["Autopilot", ask.why], ["Trade-off", ask.tradeoff], ["The ask", ask.question !== ask.title ? ask.question : null]].filter(([, text]) => text);
  return (
    <li className="auto"><Icon name={name} size="who" label={label} />
      <div>{head}
        {ask.confidence ? <p className="sure"><Confidence level={ask.confidence} bars={ask.bars} /><span><b>{ask.confidence}</b> {confidenceNote(ask.answerer === "auto" ? level : null)}</span></p> : null}
        {why.length ? <dl className="why">{why.map(([term, text]) => <Reason key={term} term={term} text={text} />)}</dl> : null}
        <p className="course"><span>{courseNote(ask.answerer, ask.confidence)}</span>{live ? <a href="#say">Say to the agent</a> : null}<a href={`/asks/${encodeURIComponent(ask.id)}`}>{`Open ask ${ask.id}`}</a></p></div><Time iso={ask.at} /></li>
  );
});

const Reason = memo(function Reason({ term, text }) {
  return <><dt>{term}</dt><dd>{text}</dd></>;
});

// The task's answered asks, newest first, each with who answered.
const AnsweredAsks = memo(function AnsweredAsks({ task }) {
  const rows = answeredAsks(task.asks || [], task.id);
  if (!rows.length) return null;
  return (
    <section className="answered" aria-labelledby="answered-head">
      <div className="ah"><h2 id="answered-head">Answered asks</h2><span className="meta">{`${rows.length}${rows.length > 1 ? ", newest first" : ""}`}</span></div>
      <ol className="arows">{rows.map((ask) => <Answered key={ask.id} ask={ask} live={!ENDED.includes(task.state)} level={task.auto_answer} />)}</ol>
    </section>
  );
});

// React keeps the textarea across refreshes, so its focus, caret and selection need no care.
const Say = memo(function Say({ task }) {
  const { busy, drafts } = useStore(acts);
  const text = drafts.get(task) ?? "";
  const submit = (event) => {
    event.preventDefault();
    if (text.trim() && busy === null) writeTask(task, "say", { text: text.trim() });
  };
  return (
    <form className="writeform" onSubmit={submit}>
      <label className="sr" htmlFor="say">Say to the agent</label>
      <textarea id="say" name="text" rows="2" placeholder="Say to the agent" value={text} onChange={(event) => draft(task, event.target.value)} />
      <button type="submit" className="btn primary" aria-disabled={busy === task ? "true" : undefined}>Say</button>
    </form>
  );
});

const Terminal = memo(function Terminal({ task }) {
  const run = terminalRun(task);
  const end = run?.ended ? run.end || "moved" : "";
  if (!run) return null;
  return (
    <Section id="terminal" icon="terminal" title="Terminal" meta={<StateWord state={end || "running"} />}>
      <TerminalPane run={run.id} task={task.id} tmux={run.tmux} end={end} />
    </Section>
  );
});

export const TaskBody = memo(function TaskBody({ task, asks, tasks }) {
  return (
    <>
      <div className="ask task">
        <p className="ctx"><Tile state={task.state} size="sm" /><span className="mono">{taskLabel(task.key)}</span><span>{task.state}</span><span>{workStep(task)}</span><span>{task.lane || ""}</span><span>{task.band || ""}</span>{task.batch == null ? null : <a href={`/batches/${task.batch}`}>{`batch ${task.batch}`}</a>}{task.auto_answer ? <span>Answers asks itself at <b>{task.auto_answer}</b></span> : null}</p>
        <h1 className="question"><Linked text={task.tldr || task.brief?.title || task.brief?.request?.split("\n")[0] || taskLabel(task.key)} repo={task.repo} /></h1>
        {task.reason ? <p className={`text t-${role(task.state)}`}><Linked text={task.reason} repo={task.repo} /></p> : null}
        <OpenAsks asks={openAsksOf(asks, task)} />
        <TaskActs task={task.id} state={task.state} revision={task.revision} />
        {ENDED.includes(task.state) ? null : <Say task={task.id} />}
        <AnsweredAsks task={task} />
      </div>
      <TaskSections task={task} tasks={tasks} />
    </>
  );
});

const NONE = [];

export const TaskPage = memo(function TaskPage({ id }) {
  const { entry, href, size } = usePlace();
  const { data: task, error } = useTask(id);
  const asks = useAsks().data?.asks ?? NONE;
  const tasks = useTasks().data?.tasks;
  const [back, label] = backTo(entry.from ?? null);
  const loaded = Boolean(task);
  // A link from another page may name a section, which exists once the task is read.
  const hash = /#.*/.exec(href)?.[0];
  useEffect(() => {
    if (loaded && hash) toHash(hash, "auto");
  }, [loaded, hash]);
  useScroll(href, loaded);
  const top = <div className="top"><a className="back" href={back}><Icon name="chevron-left" size="s22" />{label}</a></div>;
  const body = task
    ? <div className="scroll" id="task-scroll">{error ? <p className="note" role="status">{`Could not read task ${id} again: ${error.message}. This is its last read.`}</p> : null}<TaskBody task={task} asks={asks} tasks={tasks} /><Terminal task={task} /></div>
    : <><h1 className="sr">Task {id}</h1><p className="empty">{error ? `Could not load task ${id}: ${error.message}.` : `Loading task ${id}…`}</p></>;
  return <div className={listClass(size, "list page")}>{top}{body}</div>;
});

// The scroll a person left a task at, kept by its address.
const scrolls = new Map();
function useScroll(href, loaded) {
  useLayoutEffect(() => {
    const element = document.getElementById("task-scroll");
    if (!element) return undefined;
    element.scrollTop = scrolls.get(href) ?? 0;
    const kept = () => scrolls.set(href, element.scrollTop);
    element.addEventListener("scroll", kept, { passive: true });
    return () => element.removeEventListener("scroll", kept);
  }, [href, loaded]);
}
