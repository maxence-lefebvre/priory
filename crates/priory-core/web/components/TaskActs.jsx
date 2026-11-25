// A task's actions (DESIGN.md, Task detail): Revive and Cancel arm on the first tap and send on the second.
import { memo, useState } from "react";
import { ACTS, acts, actsOf, pressTask } from "../acts.js";
import { session } from "../session.js";
import { useStore } from "../hooks/useData.js";

// The countdown starts where the arming tap left it, and a redraw never restarts it.
const Drain = memo(function Drain({ at }) {
  const [delay] = useState(() => `-${Date.now() - at}ms`);
  return <span className="drain" aria-hidden="true" style={{ animationDelay: delay }} />;
});

export const TaskActs = memo(function TaskActs({ task, state }) {
  const { armed, busy, notices } = useStore(acts);
  const { failed } = useStore(session);
  const off = busy !== null || Boolean(failed);
  const notice = notices.get(task);
  return (
    <>
      <span className="task-acts">{actsOf(state).map((act) => {
        const on = armed?.task === task && armed.act === act;
        return (
          <button key={act} type="button" id={`act-${task}-${act}`} className={`btn secondary act ${on ? "armed" : ""}`} aria-disabled={off ? "true" : "false"} onClick={() => { if (!off) pressTask(task, act, {}); }}>
            {on ? <><span><small>{`Tap again to ${act}`}</small>{ACTS[act]}</span><Drain at={armed.at} /></> : ACTS[act]}
          </button>
        );
      })}</span>
      {notice ? <p className="note acts-note" role="status">{notice}</p> : null}
    </>
  );
});
