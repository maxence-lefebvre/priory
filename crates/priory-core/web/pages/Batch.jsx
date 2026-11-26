// The batch page (DESIGN.md, Batch page): /batches/<id>, its stages, its lane graph, and the issues it
// closes and splits. A go ask draws the same graph, whose entries a person can move or drop.
import { memo, useLayoutEffect, useRef, useState } from "react";
import { editBody, issueName, laneLayout, momentWords, stages, successors } from "../batches.js";
import { asked, refresh } from "../data.js";
import { plural } from "../desk.js";
import { role } from "../icons.js";
import { backTo } from "../pages.js";
import { onSignOut, post, writeRefusal } from "../session.js";
import { store } from "../store.js";
import { taskLabel } from "../tasks.js";
import { age, ago } from "../times.js";
import { Icon, Tile } from "../components/Icon.jsx";
import { NotReady } from "../components/NotReady.jsx";
import { Linked, Time } from "../components/Text.jsx";
import { useBatch } from "../hooks/useBatch.js";
import { useStore, useTasks } from "../hooks/useData.js";
import { useKeptScroll } from "../hooks/useKeptScroll.js";
import { useOlder } from "../hooks/useOlder.js";
import { listClass, usePlace } from "./place.js";

// The go ask's plan edits: the entry whose menu is open and the drop armed by its first tap, both for
// one batch at one plan revision, whether an edit is in flight, and what the last one said.
const EMPTY = () => ({ at: null, batch: null, open: null, armed: null, busy: false, note: null });
export const planEdits = store(EMPTY());
onSignOut(() => planEdits.set(EMPTY()));

const planEditAt = (batch) => `${batch.id}:${batch.plan_revision}`;
function planEditOf(edit, batch) {
  if (edit.at === planEditAt(batch)) return edit;
  return { open: null, armed: null, busy: false, note: edit.batch === batch.id ? edit.note : null };
}
/** Leaving the go ask closes an entry's menu and disarms its Drop. */
export const closePlanEdit = () => planEdits.set({ ...planEdits.get(), open: null, armed: null });

function toggleMenu(batch, entry) {
  const edit = planEditOf(planEdits.get(), batch);
  planEdits.set({ ...edit, at: planEditAt(batch), batch: batch.id, open: edit.open === entry ? null : entry, armed: null });
}

// One tap of an entry's menu. A drop that takes successors with it arms first and names them.
async function editPlan(batch, entry, action) {
  const edit = planEdits.get();
  if (edit.at !== planEditAt(batch) || edit.busy) return;
  if (action === "drop" && successors(batch.entries, entry).length && edit.armed !== entry) return planEdits.set({ ...edit, armed: entry });
  const body = editBody(batch.plan_revision, batch.entries, entry, action);
  if (!body) return;
  planEdits.set({ ...edit, busy: true });
  const { status, body: reply } = await post(`/api/batches/${batch.id}/edit`, body).catch((error) => ({ status: 0, body: { error: error.message } }));
  const note = status === 200 ? reply.ok : status === 409 ? "The plan changed before this edit. Read the new revision and edit again." : writeRefusal(status, reply);
  planEdits.set({ ...EMPTY(), batch: batch.id, note });
  refresh();
}

// An id no other control's parts can spell, so focus finds the same button after a refresh.
const focusId = (...parts) => encodeURIComponent(JSON.stringify(parts));
const NONE = [];

export const LaneKey = () => <p className="key-line"><span className="moment"><i className="mk stack" aria-hidden="true" />stacks: starts once the one above waits on review</span><span className="moment"><i className="mk" aria-hidden="true" />starts once the one above merges</span></p>;

// Lines between the rows' nodes: down the predecessor's lane, then into the entry's lane just above it.
const LANE = 18, LANE_PAD = 14, NODE_Y = 19;
function useLanes(graph, edges) {
  const [drawn, setDrawn] = useState(null);
  const shape = JSON.stringify(edges);
  useLayoutEffect(() => {
    const element = graph.current;
    const measure = () => {
      const rows = [...element.querySelectorAll(":scope > li")];
      const x = (row) => LANE_PAD + Number(row.dataset.lane) * LANE;
      const y = (row) => row.offsetTop + NODE_Y;
      const paths = JSON.parse(shape).map(({ from, to, moment }) => {
        const [x1, y1, x2, y2] = [x(rows[from]), y(rows[from]), x(rows[to]), y(rows[to])];
        return { moment, d: x1 === x2 ? `M${x1} ${y1}V${y2}` : `M${x1} ${y1}V${y2 - 40}C${x1} ${y2 - 16} ${x2} ${y2 - 24} ${x2} ${y2}` };
      });
      const next = { paths, height: element.offsetHeight };
      setDrawn((held) => (JSON.stringify(held) === JSON.stringify(next) ? held : next));
    };
    measure();
    // Rows reflow without a draw, as when the width changes or a font loads.
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [graph, shape]);
  return drawn;
}

const Lanes = memo(function Lanes({ drawn, width }) {
  if (!drawn) return null;
  return (
    <svg className="lanes" aria-hidden="true" width={width} height={drawn.height}>{drawn.paths.map(({ d, moment }, at) => (
      moment === "waiting" ? [<path key={`${at}h`} className="halo" d={d} />, <path key={`${at}s`} className="stack" d={d} />, <path key={`${at}i`} className="stack-in" d={d} />]
        : [<path key={`${at}h`} className="halo" d={d} />, <path key={`${at}b`} className="base" d={d} />]
    ))}</svg>
  );
});

const EditItem = memo(function EditItem({ batch, entry, action, glyph, label, off }) {
  return <button type="button" className="btn secondary" id={focusId("batch-edit", batch.id, entry, action)} aria-disabled={off ? "true" : "false"} onClick={() => { if (!off) editPlan(batch, entry, action); }}><Icon name={glyph} size="s15" />{label}</button>;
});

const EntryRow = memo(function EntryRow({ batch, entry, lane, at, task, editable, edit }) {
  const entries = batch.entries;
  const pr = task?.stack?.at(-1);
  const status = task ? [pr && `#${pr.number} ${pr.state}`, task.reason || [task.step, age(task.since)].filter(Boolean).join(", ")].filter(Boolean).join(", ") : "";
  const open = editable && edit.open === entry.entry;
  const armed = open && edit.armed === entry.entry;
  const off = (action) => edit.busy || (action !== "drop" && !editBody(0, entries, entry.entry, action));
  const after = armed ? successors(entries, entry.entry).map((other) => issueName(entries.find((each) => each.entry === other))) : [];
  const name = issueName(entry);
  return (
    <li className={`erow ${entry.stage === "dropped" ? "dropped" : ""}`} data-lane={lane} style={{ "--lane": String(lane) }}>
      <span className="slot">{entry.state ? <Tile state={entry.state} size="sm" /> : <span className="node num">{at + 1}</span>}</span>
      <span className="l1">{entry.task ? <a className="mono" href={`/tasks/${entry.task}`} title={`task ${entry.task}`}>{name}</a> : <span className="mono">{name}</span>}{entry.part != null ? <span className="tag">split</span> : null}{entry.task ? null : <span>{(entry.files || []).join(", ")}</span>}{entry.state ? <span className={`sti t-${role(entry.state)}`}>{entry.state}</span> : null}</span>
      <span className="l2">{entry.title}</span>
      <span className="l3"><span>{momentWords(entry, entries)}</span>{status ? <span><Linked text={status} repo={batch.repo} /></span> : null}</span>
      {editable ? <button type="button" className="iconbtn menu" id={focusId("batch-edit", batch.id, entry.entry, "menu")} aria-expanded={open ? "true" : "false"} aria-label={`Edit ${name}`} onClick={() => toggleMenu(batch, entry.entry)}><Icon name="more" size="s20" /></button> : null}
      {open ? (
        <span className="emenu">
          <EditItem batch={batch} entry={entry.entry} action="up" glyph="arrow-up" label="Move up" off={off("up")} />
          <EditItem batch={batch} entry={entry.entry} action="down" glyph="arrow-down" label="Move down" off={off("down")} />
          {armed
            ? <button type="button" className="btn secondary armed" id={focusId("batch-edit", batch.id, entry.entry, "drop")} onClick={() => editPlan(batch, entry.entry, "drop")}><Icon name="minus" size="s15" />{`Tap again to drop it and ${after.join(", ")}`}</button>
            : <EditItem batch={batch} entry={entry.entry} action="drop" glyph="minus" label="Drop" off={off("drop")} />}
        </span>
      ) : null}
    </li>
  );
});

/** The lane graph (DESIGN.md, Lane graph): one row per entry in start order, the lines drawn once the rows are laid out. */
export const LaneGraph = memo(function LaneGraph({ batch, editable }) {
  const { size } = usePlace();
  const tasks = useTasks(useStore(asked)).data?.tasks ?? NONE;
  const edit = planEditOf(useStore(planEdits), batch);
  const { rows, lanes, edges } = laneLayout(batch.entries, size === "compact" ? 3 : Infinity);
  const graph = useRef(null);
  const drawn = useLanes(graph, edges);
  const width = LANE_PAD * 2 + (lanes - 1) * LANE;
  return (
    <ol className="bgraph" ref={graph} data-lanes={lanes} data-edges={JSON.stringify(edges)} aria-label="Entries in start order" style={{ "--gw": `${width}px` }}>
      <Lanes drawn={drawn} width={width} />
      {rows.map(({ entry, lane }, at) => (
        <EntryRow key={entry.entry} batch={batch} entry={entry} lane={lane} at={at} task={entry.task ? tasks.find((each) => each.id === entry.task) : null} editable={editable} edit={edit} />
      ))}
    </ol>
  );
});

/** The go ask's plan note: what the last edit said. */
export const PlanNote = memo(function PlanNote({ batch }) {
  const { note } = planEditOf(useStore(planEdits), batch);
  return note ? <p className="note" role="status">{note}</p> : null;
});

const StageBar = memo(function StageBar({ batch }) {
  const parts = stages(batch.counts);
  const closed = (batch.closes || []).filter((close) => close.closed).length;
  return (
    <>
      <div className="stagebar" aria-hidden="true">{parts.map(([stage, count]) => <i key={stage} className={`s-${stage}`} data-grow={count} style={{ flexGrow: String(count) }} />)}</div>
      <p className="stagewords">{parts.map(([stage, count]) => <span key={stage}><b>{count}</b>{` ${stage}`}</span>)}{closed ? <span><b>{closed}</b> closed at the go</span> : null}</p>
    </>
  );
});

export const Closes = memo(function Closes({ batch }) {
  const closes = batch.closes || [];
  if (!closes.length) return null;
  return (
    <section className="sect" id="closes"><div className="sh"><Icon name="check" size="s15" />{`Closes ${closes.length} ${closes.length > 1 ? "issues" : "issue"}`}</div>
      <ol className="ilist">{closes.map((close) => (
        <li key={close.issue}><Icon name={close.closed ? "check" : "circle-dashed"} size="s15" />
          <span><span className="mono">{close.issue}</span><Linked text={` ${close.why}${close.evidence ? ` ${close.evidence}` : ""}${close.closed ? ". Closed " : ""}`} repo={batch.repo} />{close.closed ? <><Time iso={close.closed} text={ago(close.closed)} />.</> : null}</span></li>
      ))}</ol>
    </section>
  );
});

/** The plan's splits as the planner sent them, each kept only when it names an issue. */
export const splitsOf = (batch) => (batch.splits || []).filter((split) => typeof split?.issue === "string");

export const Splits = memo(function Splits({ batch }) {
  const splits = splitsOf(batch);
  if (!splits.length) return null;
  const parts = (issue) => batch.entries.filter((entry) => entry.issue === issue).map(issueName).join(", ");
  return (
    <section className="sect" id="splits"><div className="sh"><Icon name="layers" size="s15" />{`Splits ${splits.length} ${splits.length > 1 ? "issues" : "issue"}`}</div>
      <ol className="ilist">{splits.map((split) => (
        <li key={split.issue}><Icon name="layers" size="s15" /><span><span className="mono">{split.issue}</span><Linked text={` into ${parts(split.issue)}. ${split.why || ""}`} repo={batch.repo} /></span></li>
      ))}</ol>
    </section>
  );
});

/** The batch's own page, by its id. */
export const BatchPage = memo(function BatchPage({ id }) {
  const { entry, size, href } = usePlace();
  const tasks = useTasks(useStore(asked));
  useOlder("tasks", false);
  const { data: batch, error } = useBatch(id);
  const [back, label] = backTo(entry.from ?? null);
  const scroll = useKeptScroll(`batch ${href}`, Boolean(batch));
  const top = <div className="top"><a className="back" href={back}><Icon name="chevron-left" size="s22" />{label}</a></div>;
  // The lane graph names each entry's task: the page draws once the tasks are read.
  if (!tasks.data) return size === "compact" ? <div className="flat"><NotReady results={[tasks]} /></div> : <NotReady results={[tasks]} />;
  if (!batch) {
    return (
      <div className={listClass(size, "list page")}>{top}<h1 className="sr">{`Batch ${id}`}</h1>
        <p className="empty">{error ? <>{`Could not load batch ${id}: ${error.message}. `}<code>{`priory batch show ${id}`}</code> shows it.</> : `Loading batch ${id}…`}</p></div>
    );
  }
  return (
    <div className={listClass(size, "list page")}>{top}
      <div className="scroll" ref={scroll}><div className="ask bhead">
        <p className="ctx"><Tile state="batch" size="sm" /><a className="mono" href={`/tasks/${batch.task}`} title={`task ${batch.task}`}>{taskLabel(batch.key)}</a><span>{batch.state}</span><span>{batch.lane || ""}</span>{batch.went ? <span>went <Time iso={batch.went} text={ago(batch.went)} />{` by ${batch.went_by}`}</span> : null}<span>{`plan rev ${batch.plan_revision}`}</span></p>
        <h1 className="question">{batch.title || taskLabel(batch.key)}</h1><StageBar batch={batch} /></div>
        <section className="sect" id="entries"><div className="sh"><Icon name="layers" size="s15" />{`${plural(batch.entries.length, "entry", "entries")} in start order`}</div><LaneKey /><LaneGraph batch={batch} editable={false} /></section>
        <Closes batch={batch} /><Splits batch={batch} /></div>
    </div>
  );
});
