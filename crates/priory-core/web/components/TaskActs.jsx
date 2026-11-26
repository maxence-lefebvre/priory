// A task's actions (DESIGN.md, Task detail): Revive and Cancel arm on the first tap and send on the second.
import { memo, useState } from "react";
import { ACTS, acts, actsOf, pressTask } from "../acts.js";
import { session } from "../session.js";
import { useStore } from "../hooks/useData.js";

// The countdown starts where the arming tap left it, and a redraw never restarts it.
export const Drain = memo(function Drain({ at }) {
  const [delay] = useState(() => `-${Date.now() - at}ms`);
  return <span className="drain" aria-hidden="true" style={{ animationDelay: delay }} />;
});

/**
 * The actions of a task in `state`, or the `only` ones given, such as the Inbox's Resume beside its Say.
 * Another runner's task, `runner:key`, sends the `generation` the list drew instead of a revision.
 * `prefix` keeps a second copy's ids apart.
 */
export const TaskActs = memo(function TaskActs({ task, state, only, revision, generation, prefix = "" }) {
  const { armed, busy, notices } = useStore(acts);
  const { failed } = useStore(session);
  const remote = typeof task === "string";
  const off = busy !== null || Boolean(failed) || (!remote && revision === undefined);
  const notice = notices.get(task);
  return (
    <>
      <span className="task-acts">{(only ?? actsOf(state)).map((act) => {
        const on = armed?.task === task && armed.act === act;
        return (
          <button key={act} type="button" id={`${prefix}act-${task}-${act}`} className={`btn secondary act ${on ? "armed" : ""}`} aria-disabled={off ? "true" : "false"} onClick={() => { if (!off) pressTask(task, act, remote ? { generation, body: { generation } } : { revision }); }}>
            {on ? <><span><small>{`Tap again to ${act}`}</small>{ACTS[act]}</span><Drain at={armed.at} /></> : ACTS[act]}
          </button>
        );
      })}</span>
      {notice ? <p className="note acts-note" role="status">{notice}</p> : null}
    </>
  );
});
