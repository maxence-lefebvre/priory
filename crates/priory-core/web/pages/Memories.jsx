// Memory candidates (DESIGN.md, Memory candidates): each candidate's full text, then Accept and Drop.
import { memo, useEffect, useState } from "react";
import { ARM_MS } from "../asks.js";
import { Icon, Tile } from "../components/Icon.jsx";
import { Time } from "../components/Text.jsx";
import { refresh } from "../data.js";
import { memoryNote, memoryTag, plural } from "../desk.js";
import { useDesk, useStore } from "../hooks/useData.js";
import { post, refuse, session } from "../session.js";
import { store } from "../store.js";
import { listClass, usePlace } from "./place.js";

// The Accept armed by its first tap { id, at, timer }, the one in flight, each candidate's last note,
// those sent, whose buttons stay off until the desk drops them, and those whose Current text is open,
// so a refresh keeps it open.
export const memoryActs = store({ armed: null, busy: null, notices: new Map(), sent: new Set(), open: new Set() });
const change = (part) => memoryActs.set({ ...memoryActs.get(), ...part });

export function disarmMemory() {
  const { armed } = memoryActs.get();
  if (!armed) return;
  clearTimeout(armed.timer);
  change({ armed: null });
}

/** A tap on Accept or Drop. Accept arms on the first tap and sends on the second, as Cancel does; Drop sends at once. */
export async function pressMemory(id, verb) {
  const armed = memoryActs.get().armed?.id === id;
  disarmMemory();
  if (verb === "accept" && !armed) return change({ armed: { id, at: Date.now(), timer: setTimeout(() => change({ armed: null }), ARM_MS) } });
  change({ busy: id });
  let reply;
  try {
    reply = await post(`/api/memories/${encodeURIComponent(id)}/${verb}`, {});
  } catch {
    reply = { status: 0, body: { error: "the connection dropped" } };
  }
  const { status, body } = reply;
  if (status === 401) {
    change({ busy: null });
    return refuse();
  }
  const sent = new Set(memoryActs.get().sent);
  if (status === 200) sent.add(id);
  const notices = new Map(memoryActs.get().notices).set(id, memoryNote(verb, status, body));
  change({ busy: null, sent, notices });
  refresh();
}

function toggleCurrent(event, id) {
  event.preventDefault();
  const open = new Set(memoryActs.get().open);
  if (!open.delete(id)) open.add(id);
  change({ open });
}

export const Memories = memo(function Memories() {
  const { size } = usePlace();
  const candidates = useDesk().data?.items.find((item) => item.kind === "memories")?.candidates || [];
  // An Accept armed here does not outlive the page.
  useEffect(() => disarmMemory, []);
  const n = candidates.length;
  return (
    <div className={listClass(size, "list page")}>
      <div className="top"><a className="back" href="/"><Icon name="chevron-left" size="s22" />Desk</a></div>
      <div className="scroll">
        <div className="ask mhead"><h1 className="question">{plural(n, "memory candidate")}</h1>
          <p className="note"><Icon name="brain" size="s15" /><span>An agent wrote each one. Accept gives it to every task&apos;s runs, on every runner. Drop discards it, and the task that wrote it stops getting it.</span></p></div>
        {n ? candidates.map((candidate) => <Candidate key={candidate.id} candidate={candidate} />) : <p className="empty">No memory candidate waits. Agents write them with the remember and forget tools.</p>}
      </div>
    </div>
  );
});

const Candidate = memo(function Candidate({ candidate }) {
  const { open } = useStore(memoryActs);
  const tag = memoryTag(candidate);
  return (
    <section className="sect mem" aria-label={candidate.name}>
      <div className="sh"><Tile state="memories" size="sm" /><span className="mono">{candidate.name}</span><span className={`tag ${tag.warn ? "t-warn" : ""}`}>{tag.label}</span><span className="meta"><Time iso={candidate.written} /></span></div>
      {candidate.forget
        ? <p className="text">{`Forgets ${candidate.name}: runs stop getting it.`}</p>
        : <><p className="desc">{candidate.description}</p><p className="text">{candidate.text}</p></>}
      {candidate.current_text == null ? null
        : <details className="current" open={open.has(candidate.id)}><summary onClick={(event) => toggleCurrent(event, candidate.id)}>Current text</summary><p className="text">{candidate.current_text}</p></details>}
      <p className="by">Written by <span className="mono">{candidate.task}</span>{candidate.runner ? ` on ${candidate.runner}` : ""}, candidate <span className="mono">{candidate.id}</span></p>
      <Acts id={candidate.id} />
    </section>
  );
});

// The countdown starts where the arming tap left it, and a redraw never restarts it.
const Drain = memo(function Drain({ at }) {
  const [delay] = useState(() => `-${Date.now() - at}ms`);
  return <span className="drain" aria-hidden="true" style={{ animationDelay: delay }} />;
});

const Acts = memo(function Acts({ id }) {
  const { armed, busy, sent, notices } = useStore(memoryActs);
  const { failed } = useStore(session);
  const off = busy !== null || sent.has(id) || Boolean(failed);
  const on = armed?.id === id;
  const button = (verb, kind, label) => (
    <button type="button" id={`memory-${verb}-${id}`} className={`btn ${kind} act ${verb === "accept" && on ? "armed" : ""}`} aria-disabled={off ? "true" : "false"} onClick={() => { if (!off) pressMemory(id, verb); }}>{label}</button>
  );
  const notice = notices.get(id);
  return (
    <>
      <span className="task-acts">
        {button("accept", "primary", on ? <><span><small>Tap again to accept</small>Accept</span><Drain at={armed.at} /></> : "Accept")}
        {button("drop", "secondary", "Drop")}
      </span>
      {notice ? <p className="note acts-note" role="status"><Icon name="corner-down-right" size="s15" /><span>{notice}</span></p> : null}
    </>
  );
});
