// Brief, Progress, Plan, PR, Inbox and Trace of a task, each left out when the task lacks it.
import { memo } from "react";
import { planParts } from "../asks.js";
import { progressView } from "../desk.js";
import { role } from "../icons.js";
import { eventWords, handled } from "../inbox.js";
import { tokenText } from "../runs.js";
import { checkState, prStack, stateWords } from "../stack.js";
import { taskLabel, triageWords } from "../tasks.js";
import { useMinute } from "../hooks/useScreen.js";
import { Icon, Tile } from "./Icon.jsx";
import { Linked, Markdown, Time } from "./Text.jsx";

export const Section = memo(function Section({ id, icon, title, meta, children }) {
  return <section className="sect" id={id}><div className="sh"><Icon name={icon} size="s15" />{title}<span className="meta">{meta}</span></div>{children}</section>;
});

const Brief = memo(function Brief({ task }) {
  const { request, result } = task.brief || {};
  if (!request && !result) return null;
  return (
    <Section id="brief" icon="file-text" title="Brief" meta={result ? `${result.step}, run ${result.run}` : ""}>
      {request ? <><p className="meta">The request</p><Markdown text={request} repo={task.repo} /></> : null}
      {result ? <Markdown text={result.summary} repo={task.repo} /> : null}
    </Section>
  );
});

const Line = memo(function Line({ label, text, repo }) {
  return text ? <><p className="meta">{label}</p><p className="text"><Linked text={text} repo={repo} /></p></> : null;
});

const Progress = memo(function Progress({ task }) {
  const progress = progressView(task.progress);
  if (!progress) return null;
  return (
    <Section id="progress" icon="check" title="Progress" meta="">
      <Line label="Goal" text={progress.goal} repo={task.repo} />
      {progress.criteria.length ? (
        <><p className="meta">Acceptance criteria</p><ol className="hist crit">{progress.criteria.map((criterion) => (
          <li key={criterion.id}><span className="mono">{criterion.id}</span><span><Linked text={criterion.description} repo={task.repo} /></span><span className="tag">{criterion.state}</span></li>
        ))}</ol></>
      ) : null}
      <Line label="Latest change" text={progress.latestChange} repo={task.repo} />
      <Line label="Next" text={progress.nextAction} repo={task.repo} />
    </Section>
  );
});

const PlanMeta = memo(function PlanMeta({ part }) {
  return <span className="mono" title={part.path || ""}>{part.from}</span>;
});

const PlanBody = memo(function PlanBody({ part, repo }) {
  return (
    <>
      <Markdown text={part.text} repo={repo} />
      {part.truncated ? <p className="note"><Icon name="file-text" size="s15" /><span>{`Showing the first 64 KB. Read the rest at ${part.path || part.from}.`}</span></p> : null}
    </>
  );
});

const Plan = memo(function Plan({ task }) {
  if (task.plan_file) {
    const [first, ...others] = planParts(task.plan_file);
    return (
      <Section id="plan" icon="check" title="Plan" meta={<PlanMeta part={first} />}>
        <PlanBody part={first} repo={task.repo} />
        {others.map((part) => (
          <PlanPart key={part.path || part.from} part={part} repo={task.repo} />
        ))}
      </Section>
    );
  }
  if (!task.plan) return null;
  return <Section id="plan" icon="check" title="Plan" meta={`${task.plan.step}, run ${task.plan.run}`}><Markdown text={task.plan.summary} repo={task.repo} /></Section>;
});

const PlanPart = memo(function PlanPart({ part, repo }) {
  return <><div className="sh"><Icon name="check" size="s15" />Plan<span className="meta"><PlanMeta part={part} /></span></div><PlanBody part={part} repo={repo} /></>;
});

const PULL_TILES = { merged: "done", closed: "dropped" };

const Stack = memo(function Stack({ stack, latest }) {
  const { label, entries } = prStack(stack, latest);
  if (!label) return null;
  return (
    <><p className="meta">{`Stack, bottom first: ${label}`}</p><ol className="hist" aria-label="Pull requests, bottom first">{entries.map((pull) => (
      <li key={pull.url}>
        <Tile state={PULL_TILES[pull.state] || (pull.next ? "ready" : "queued")} size="sm" />
        <span><a className="autolink mono" href={pull.url} rel="noopener noreferrer" target="_blank">{`#${pull.number}`}</a>{` ${pull.branch} into ${pull.base}${pull.checks ? `, ${pull.checks}` : ""}`}</span>
        <span className={pull.next ? "t-live" : ""}>{pull.next ? `${pull.state}, next to merge` : pull.state}</span>
      </li>
    ))}</ol></>
  );
});

const TRIAGE_TILES = { trivial: "ready", "nothing-to-do": "dropped", "persons-call": "ask" };
const named = (item) => (item.startsWith("https://") ? <a className="autolink" href={item} rel="noopener noreferrer" target="_blank">{item.replace(/^https:\/\/github\.com\//, "")}</a> : item);

// How the agent sorted each review feedback item: trivial, nothing-to-do or persons-call.
const Triage = memo(function Triage({ triage }) {
  if (!triage?.length) return null;
  return (
    <><p className="meta">Feedback, as the agent sorted it</p><ol className="hist pair" aria-label="Review feedback items">{triage.map((entry) => (
      <li key={entry.item}><Tile state={TRIAGE_TILES[entry.class] || "queued"} size="sm" /><span>{named(entry.item)}</span><span>{triageWords(entry.class)}</span></li>
    ))}</ol></>
  );
});

const PullRequest = memo(function PullRequest({ task, stack }) {
  const pr = task.pull_request;
  if (pr) {
    return (
      <Section id="pr" icon="workflow" title={`#${pr.number} ${pr.title || ""}`} meta={stateWords(pr.review)}>
        <p className="meta">{`${pr.branch} into ${pr.base}, head `}<span className="mono">{String(pr.head_sha).slice(0, 7)}</span>{`, ${pr.state}`}</p>
        <ol className="hist pair">{pr.checks.map((check) => (
          <li key={check.name}><Tile state={checkState(check.conclusion)} size="sm" /><span>{check.name}</span><span>{check.conclusion ? stateWords(check.conclusion) : "running"}</span></li>
        ))}</ol>
        <Stack stack={task.stack} latest={pr} /><Triage triage={task.triage} />
      </Section>
    );
  }
  // The summary needs the pull request's events; the stack does not.
  const { label } = prStack(stack);
  if (label) return <Section id="pr" icon="workflow" title={label} meta=""><Stack stack={task.stack} latest={null} /><Triage triage={task.triage} /></Section>;
  return task.triage?.length ? <Section id="pr" icon="workflow" title="Feedback" meta=""><Triage triage={task.triage} /></Section> : null;
});

const INBOX_SHOWN = 20;
// A prompt's state as a tile: delivered, waiting to be typed, being typed, or withdrawn.
const INBOX_TILES = { confirmed: "done", queued: "queued", attempted: "running", uncertain: "prompt", withdrawn: "dropped" };

const Inbox = memo(function Inbox({ task, tasks }) {
  if (!task.inbox?.length) return null;
  const nameOf = (id) => {
    const other = tasks?.find((each) => each.id === id);
    return other ? taskLabel(other.key) : `task ${id}`;
  };
  return (
    <Section id="inbox" icon="message-text" title="Inbox" meta="newest first">
      <ol className="hist" aria-label="Events routed to this task, newest first">{task.inbox.slice(0, INBOX_SHOWN).map((event) => (
        <li key={event.id ?? `${event.time} ${event.type} ${event.subject}`}>
          <Tile state={INBOX_TILES[event.prompt] || (event.outcome === "kept" && event.task === task.id ? "asleep" : "ready")} size="sm" />
          <span><b title={event.type}>{eventWords(event.type)}</b>{` ${event.subject || ""}`}<span className="meta">{handled(event, task.id, nameOf)}</span></span><Time iso={event.time} />
        </li>
      ))}</ol>
      {task.inbox.length > INBOX_SHOWN ? <p className="meta">{`Latest ${INBOX_SHOWN} of ${task.inbox.length}. `}<code>{`priory task ${task.id}`}</code> lists every event.</p> : null}
    </Section>
  );
});

const Run = memo(function Run({ run, scores }) {
  useMinute();
  const tokens = tokenText(run.tokens);
  const minutes = Math.round(((run.ended ? Date.parse(run.ended) : Date.now()) - Date.parse(run.started)) / 60000);
  return (
    <li>
      <Tile state={run.ended ? run.end || "moved" : "running"} size="sm" />
      <span><b>{run.step}</b>, run <span className="mono">{run.id}</span>{`, ${minutes} min${run.outcome ? `, ${run.outcome}` : ""}${tokens ? `, ${tokens} tokens` : ""}`}
        {scores.length ? <ol className="scores" aria-label={`Scores of run ${run.id}`}>{scores.map((score) => (
          <li key={score.scorer}><Tile state={score.verdict} size="sm" /><span><span className="mono">{score.scorer}</span> <span className={`v t-${role(score.verdict)}`}>{score.verdict}</span><span className="meta">{score.reason}</span></span></li>
        ))}</ol> : null}
      </span><Time iso={run.started} />
    </li>
  );
});

const Trace = memo(function Trace({ task }) {
  if (!task.runs?.length) return null;
  return (
    <Section id="trace" icon="runs" title="Trace" meta="">
      <ol className="hist">{task.runs.map((run) => <Run key={run.id} run={run} scores={(task.scores || []).filter((score) => score.run === String(run.id))} />)}</ol>
    </Section>
  );
});

const stackOf = (task, tasks) => task.stack ?? tasks?.find((each) => each.id === task.id)?.stack;

/** The sections a task draws, each [key, label], in their order. */
export function sectionsOf(task, tasks) {
  const pr = task.pull_request || prStack(stackOf(task, tasks)).label ? "PR" : task.triage?.length ? "Feedback" : null;
  return [
    (task.brief?.request || task.brief?.result) && ["brief", "Brief"],
    progressView(task.progress) && ["progress", "Progress"],
    (task.plan_file || task.plan) && ["plan", "Plan"],
    pr && ["pr", pr],
    task.inbox?.length > 0 && ["inbox", "Inbox"],
    task.runs?.length > 0 && ["trace", "Trace"],
  ].filter(Boolean);
}

/** `tasks` is the tasks listing, which names the other tasks and holds a stack the detail lacks. `only` keeps those keys. */
export const TaskSections = memo(function TaskSections({ task, tasks, only }) {
  const stack = stackOf(task, tasks);
  const shows = (key) => !only || only.includes(key);
  return <>{shows("brief") ? <Brief task={task} /> : null}{shows("progress") ? <Progress task={task} /> : null}{shows("plan") ? <Plan task={task} /> : null}{shows("pr") ? <PullRequest task={task} stack={stack} /> : null}{shows("inbox") ? <Inbox task={task} tasks={tasks} /> : null}{shows("trace") ? <Trace task={task} /> : null}</>;
});
