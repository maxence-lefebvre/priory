// Reports (DESIGN.md, Reports panel): the report written last, its sections in its own order, picks
// with an Undo, and threads on rows through the report endpoints.
import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";
import { cache, refresh } from "../data.js";
import { sectionInView } from "../desk.js";
import { keyOf } from "../queries.js";
import { around, dossier, nextPick, refusal } from "../reports.js";
import { onSignOut, post, refuse, session } from "../session.js";
import { store } from "../store.js";
import { taskLabel } from "../tasks.js";
import { Icon, Tile } from "../components/Icon.jsx";
import { PageTop, Panel } from "../components/PhonePage.jsx";
import { Linked } from "../components/Text.jsx";
import { useReport, useReports, useStore, useTasks } from "../hooks/useData.js";
import { useMinute } from "../hooks/useScreen.js";
import { askFor, asked } from "../legacy/data.js";
import { writeRefusal } from "../legacy/shared.js";
import { listClass, usePlace } from "./place.js";

// What the page holds beyond the reads, kept from one visit to the next: the pick waiting out its
// Undo, then in flight; each row's last refusal; the search text, null while it is closed; the row
// whose agreed pick Change reopened; a pick's or a comment's outcome once its row has left the report;
// the comment field open on a row, with its text; whether a comment verb is in flight; the report's task.
const FRESH = () => ({ pending: null, notices: new Map(), query: null, changing: null, flash: null, compose: null, remarking: false, task: undefined });
const page = store(FRESH());
const set = (part) => page.set({ ...page.get(), ...part });
// The row Next pick reached last.
let reached = null;
onSignOut(() => {
  clearTimeout(page.get().pending?.timer);
  page.set(FRESH());
});

// An id no other control's parts can spell, so focus finds the same button after a refresh.
const focusId = (...parts) => encodeURIComponent(JSON.stringify(parts));
const drawn = () => cache.getQueryData(keyOf("report", asked.get()));
const rowOf = (read, id) => (read ? dossier(read, { now: Date.now() }).sections.flatMap((section) => section.rows).find((row) => row.id === id) : null);
const offline = () => Boolean(session.get().failed);

// A note under its row, or above the sections once the row has left the report or another task's
// report replaced it.
function noteRow(id, text, task) {
  const read = drawn();
  if (task === read?.report.task && rowOf(read, id)) set({ notices: new Map(page.get().notices).set(id, text) });
  else set({ flash: text });
}
const unnote = (id) => {
  const notices = new Map(page.get().notices);
  notices.delete(id);
  return notices;
};

// Nothing leaves the page until the Undo time passes: a pick wakes the report's agent.
const UNDO_MS = 4000;

function pick(rowId, option) {
  const read = drawn();
  const row = rowOf(read, rowId);
  const { pending, changing } = page.get();
  const reopened = row?.pick?.state === "agreed" && changing === row.id;
  if (!row?.pick || (row.pick.state !== "open" && !reopened) || option === row.pick.chosen || pending || offline()) return;
  const next = { row: row.id, task: read.report.task, pick: row.pick.id, option, from: row.pick.chosen, revision: read.report.revision, sending: false };
  next.timer = setTimeout(() => sendPick(next), UNDO_MS);
  set({ pending: next, notices: unnote(row.id), flash: null });
}

function undoPick() {
  const { pending } = page.get();
  if (!pending || pending.sending) return;
  clearTimeout(pending.timer);
  set({ pending: null });
}

// A pick still waiting out its Undo does not go once its page is left.
function dropPick() {
  const { pending } = page.get();
  if (!pending || pending.sending) return;
  clearTimeout(pending.timer);
  set({ pending: null });
}

// A new read cancels a pick still waiting out its Undo once its report no longer offers it. Another
// task's report drops the notes, which belong to rows of the old one; the comment field waits for
// its own task's report.
function accept(read) {
  const task = read?.report.task;
  if (task !== page.get().task) set({ notices: new Map(), flash: null, task });
  const { pending } = page.get();
  if (!pending || pending.sending) return;
  const row = task === pending.task ? rowOf(read, pending.row) : null;
  if (row?.pick?.id === pending.pick && row.pick.state !== "applied" && row.pick.chosen === pending.from) return;
  clearTimeout(pending.timer);
  set({ pending: null });
  noteRow(pending.row, `${pending.option} was not sent: the report changed before it went.`, pending.task);
}

async function sendPick(pending) {
  set({ pending: { ...pending, sending: true } });
  const { status, body } = await post("/api/reports/pick", { pick: pending.pick, option: pending.option, revision: pending.revision })
    .catch(() => ({ status: 0, body: { error: "the connection dropped" } }));
  set({ pending: null, changing: null });
  if (status === 401) return refuse();
  await refresh();
  if (status === 200) return;
  const fresh = status === 409 && drawn()?.report.task === pending.task ? drawn() : null;
  noteRow(pending.row, `${pending.option}: ${writeRefusal(status, body, () => refusal(pending, fresh, body.error))}`, pending.task);
}

function compose(rowId, replyTo) {
  const task = drawn().report.task;
  const { compose: open } = page.get();
  const same = open?.row === rowId && open.task === task;
  set({ compose: { task, row: rowId, replyTo, text: same ? open.text : "" } });
}

// Sends one comment verb; a refusal says why under the row and keeps the draft. `sent` runs before
// the read that follows, so a second press cannot send the same text again.
async function remark(rowId, path, body, sent = () => {}) {
  if (page.get().remarking || offline()) return;
  const task = drawn().report.task;
  set({ remarking: true, notices: unnote(rowId) });
  const { status, body: answered } = await post(path, body).catch(() => ({ status: 0, body: { error: "the connection dropped" } }));
  if (status === 401) {
    set({ remarking: false });
    return refuse();
  }
  if (status === 200) sent();
  await refresh();
  if (status !== 200) noteRow(rowId, writeRefusal(status, answered), task);
  set({ remarking: false });
}

const onNewest = (read, reports) => asked.get().reportShown == null && read?.report.task === reports?.[0]?.task;

function submitRemark(reports) {
  const read = drawn();
  const { compose: sent, remarking } = page.get();
  const row = sent?.task === read?.report.task && rowOf(read, sent.row);
  const text = sent?.text.trim();
  if (!row?.target || !text || remarking || (sent.replyTo == null && !onNewest(read, reports))) return;
  // Text typed while it was in flight stays as the next draft.
  const done = () => {
    const now = page.get().compose;
    if (now?.row === sent.row && now.task === sent.task && now.replyTo === sent.replyTo && now.text.trim() === text) set({ compose: null });
  };
  remark(row.id, ...(sent.replyTo == null
    ? ["/api/reports/comment", { row: row.target, text, revision: read.report.revision }]
    : ["/api/reports/reply", { comment: sent.replyTo, text }]), done);
}

// The newest report is followed as it changes; an older one stays until an arrow moves on.
function showReport(task, reports) {
  askFor({ reportShown: task === reports?.[0]?.task ? null : task });
  set({ changing: null });
  reached = null;
}

function reachNextPick(model) {
  const id = nextPick(model, reached);
  if (!id) return;
  reached = id;
  const row = document.getElementById(focusId("row", id));
  row?.scrollIntoView({ block: "center", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  row?.querySelector(".rpick button")?.focus({ preventScroll: true });
}

// A disabled control says so with aria-disabled and stays in the tab order.
const unless = (off, act) => () => {
  if (!off) act();
};

const Pick = memo(function Pick({ row, pending, changing, failed }) {
  const { pick: choice } = row;
  const reopened = choice.state === "agreed" && changing === row.id;
  if (choice.state !== "open" && !reopened) {
    const change = choice.state === "agreed" && !pending && !failed
      ? <button type="button" id={focusId("change", row.id)} className="ghost" onClick={() => set({ changing: row.id })}>Change</button> : null;
    return <p className="note rpicked"><Icon name="check" size="s15" /><span><b>{`Picked ${choice.chosen}.`}</b>{` ${choice.note}`}</span>{change}</p>;
  }
  const mine = pending?.pick === choice.id ? pending : null;
  const off = Boolean(failed || (pending && !mine));
  let note = null;
  if (failed) note = <p className="note"><Icon name="unplug" size="s15" /><span>Reconnect to pick.</span></p>;
  else if (reopened) note = <p className="note"><span>{`Picked ${choice.chosen}. Pick another option to change it.`}</span>{mine ? null : <button type="button" className="ghost" onClick={() => set({ changing: null })}>{`Keep ${choice.chosen}`}</button>}</p>;
  else if (choice.note) note = <p className="note">{choice.note}</p>;
  return (
    <><div className="rpick" role="group" aria-label={`Pick for ${row.key || row.text}`}>{choice.options.map((option) => {
      const chosen = mine ? mine.option === option.label : option.label === choice.chosen;
      return <button type="button" key={option.label} id={focusId("pick", choice.id, option.label)} className={chosen ? "chosen" : ""} aria-pressed={chosen ? "true" : "false"} aria-disabled={off || mine ? "true" : "false"} onClick={unless(off || mine, () => pick(row.id, option.label))}>{chosen ? <Icon name="check" size="s15" /> : null}{option.label}</button>;
    })}</div>{note}</>
  );
});

const Entry = ({ comment }) => <li><p className="rwho"><b>{comment.author}</b>{comment.when ? <> <span>{comment.when}</span></> : null}</p><p className="rsay">{comment.text}</p></li>;

const Thread = memo(function Thread({ row, thread, off }) {
  const disabled = off ? "true" : "false";
  const verb = (action) => unless(off, () => remark(row.id, `/api/reports/${action}`, { comment: Number(thread.id) }));
  if (thread.resolved) {
    return <p className="note rresolved"><Icon name="check" size="s15" /><span>{`Resolved: ${thread.text}`}</span><button type="button" id={`reopen-${thread.id}`} className="ghost" aria-disabled={disabled} onClick={verb("reopen")}>Reopen</button></p>;
  }
  return (
    <div className="rthread" aria-label={`Thread ${thread.id}`}><ol><Entry comment={thread} />{thread.replies.map((reply) => <Entry key={reply.id} comment={reply} />)}</ol>
      <p className="racts"><button type="button" id={`reply-${thread.id}`} className="ghost" aria-disabled={disabled} onClick={unless(off, () => compose(row.id, Number(thread.id)))}>Reply</button><button type="button" id={`resolve-${thread.id}`} className="ghost" aria-disabled={disabled} onClick={verb("resolve")}>Resolve</button></p></div>
  );
});

// A row without its own id takes no comments. Priory gives a row id's new thread to the newest
// report holding it, so an older report takes replies only.
const Compose = memo(function Compose({ row, open, newest, failed, remarking, reports }) {
  if (!row.target) return null;
  if (!open && !newest) return null;
  if (!open) return <p className="racts"><button type="button" id={focusId("compose", row.id)} className="ghost" aria-disabled={failed ? "true" : "false"} onClick={unless(failed, () => compose(row.id, null))}><Icon name="message-text" size="s15" />Comment</button></p>;
  const verb = open.replyTo == null ? "Comment" : "Reply";
  const submit = (event) => {
    event.preventDefault();
    submitRemark(reports);
  };
  return (
    <form className="writeform rcompose" onSubmit={submit}>{open.replyTo == null ? null : <p className="note">{`Replying in thread ${open.replyTo}`}</p>}
      <label className="sr" htmlFor="report-draft">{`${verb} on this row`}</label>
      {/* Opening the field puts the caret in it; a refresh keeps the element, so the caret stays. */}
      {/* oxlint-disable-next-line jsx-a11y/no-autofocus */}
      <textarea id="report-draft" name="text" rows="2" autoFocus placeholder={open.replyTo == null ? "Comment on this row" : "Reply in this thread"} value={open.text} onChange={(event) => set({ compose: { ...page.get().compose, text: event.target.value } })} />
      <p className="racts"><button type="button" className="btn secondary" onClick={() => set({ compose: null })}>Cancel</button><button type="submit" className="btn primary" aria-disabled={failed || remarking ? "true" : "false"}>{verb}</button></p></form>
  );
});

const Row = memo(function Row({ row, held, failed, newest, task, reports }) {
  const facts = [row.meta, row.owner].filter(Boolean);
  const notice = held.notices.get(row.id);
  const open = held.compose?.row === row.id && held.compose.task === task ? held.compose : null;
  return (
    <li className="rrow" id={focusId("row", row.id)}>
      <p className="rl1">{row.tile ? <Tile state={row.tile} size="sm" /> : null}{row.key ? <span className="mono"><Linked text={row.key} /></span> : null}<span className="rtext"><Linked text={row.text} /></span></p>
      {facts.length || row.time ? <p className="rl2">{facts.map((fact) => <span key={fact}>{fact}</span>)}{row.time ? <span>{row.time}</span> : null}</p> : null}
      {row.pick ? <Pick row={row} pending={held.pending} changing={held.changing} failed={failed} /> : null}
      {notice ? <p className="warnline" role="status">{notice}</p> : null}
      {row.threads.map((thread) => <Thread key={thread.id} row={row} thread={thread} off={failed || held.remarking} />)}
      <Compose row={row} open={open} newest={newest} failed={failed} remarking={held.remarking} reports={reports} /></li>
  );
});

const NONE = [];

const Report = memo(function Report({ read, reports, tasks }) {
  const held = useStore(page);
  const failed = Boolean(useStore(session).failed);
  const now = useMinute() * 60000;
  const scroller = useRef(null);
  const strip = useRef(null);
  const [lit, setLit] = useState(0);
  const model = dossier(read, { now, query: held.query ?? "" });
  const count = model.sections.length;
  // The chip of the section in view.
  useEffect(() => {
    const element = scroller.current;
    if (!element || !count) return undefined;
    const light = () => {
      const sections = [...element.querySelectorAll(".sect")];
      const top = element.getBoundingClientRect().top - element.scrollTop;
      const tops = sections.map((section) => section.getBoundingClientRect().top - top);
      // A chip lands its section under the chip bar, at the section's scroll-margin-top.
      const bar = sections.length ? parseFloat(getComputedStyle(sections[0]).scrollMarginTop) || 0 : 0;
      const at = sectionInView({ tops, scrollTop: element.scrollTop, height: element.clientHeight, scrollHeight: element.scrollHeight, bar });
      if (at >= 0) setLit(at);
    };
    element.addEventListener("scroll", light, { passive: true });
    light();
    return () => element.removeEventListener("scroll", light);
  }, [count]);
  // On a phone the chips scroll sideways: the lit one stays in view.
  useEffect(() => {
    const chips = strip.current;
    const chip = chips?.children[lit];
    if (chip && chips.scrollWidth > chips.clientWidth) chips.scrollLeft = chip.offsetLeft - (chips.clientWidth - chip.offsetWidth) / 2;
  }, [lit]);
  const { agreed, total, label } = model.progress;
  const { older, newer } = around(reports ?? NONE, model.task);
  const task = tasks?.find((each) => each.id === model.task);
  const newest = onNewest(read, reports);
  // Another report opens at its top.
  useLayoutEffect(() => {
    scroller.current?.scrollTo({ top: 0 });
  }, [model.task]);
  const arrow = (to, glyph, name) => <button type="button" className="iconbtn" aria-label={name} aria-disabled={to == null ? "true" : "false"} onClick={unless(to == null, () => showReport(to, reports))}><Icon name={glyph} size="s20" /></button>;
  const { pending } = held;
  return (
    <Panel label="Reports" kind="report">
      <div className="rbar">{arrow(older, "chevron-left", "Older report")}<span className="rdate">{model.date}</span>{arrow(newer, "chevron-right", "Newer report")}<span className="sp" />
        {model.views[1].count ? <button type="button" className="btn secondary" onClick={() => reachNextPick(model)}>Next pick</button> : null}
        <button type="button" className="iconbtn" aria-label="Search this report" aria-pressed={held.query == null ? "false" : "true"} onClick={() => set({ query: held.query == null ? "" : null })}><Icon name="search" size="s20" /></button></div>
      {held.query == null ? null : (
        <div className="rsearch"><label className="sr" htmlFor="report-query">Search this report</label>
          {/* oxlint-disable-next-line jsx-a11y/no-autofocus */}
          <input id="report-query" type="search" placeholder="Search this report" value={held.query} autoComplete="off" autoFocus onChange={(event) => set({ query: event.target.value })} />
          {model.found == null ? null : <span className="num" role="status">{`${model.found} found`}</span>}</div>
      )}
      <div className="scroll" data-sections="" ref={scroller}>
        <header className="rhead"><p className="greet">{model.greeting}</p><p className="meta">{`${model.date}, `}<a className="mono" href={`/tasks/${model.task}`} title={`task ${model.task}`}>{task ? taskLabel(task.key) : `task ${model.task}`}</a>{`. ${model.since}`}</p>
          {held.flash ? <p className="warnline" role="status">{held.flash}</p> : null}
          {total ? <p className="prog"><progress max={total} value={agreed} aria-label="Picks agreed" /><span className="num">{label}</span></p> : null}</header>
        <div className="rbody"><nav className="jump" aria-label="Sections" ref={strip}>{model.sections.map((section, i) => <a key={section.id} href={`#report-${i}`} aria-current={i === lit ? "true" : "false"}>{section.title}<span className="num">{section.count}</span></a>)}</nav>
          <div>{model.found === 0 ? <p className="empty">{`Nothing in this report holds “${held.query.trim()}”.`}</p> : null}
            {model.sections.map((section, i) => (
              <section key={section.id} className="sect" id={`report-${i}`} aria-labelledby={`report-${i}-title`}>
                <h2 className="sh" id={`report-${i}-title`}>{section.title}<span className="meta num">{section.count}</span></h2>
                <ol className="rrows">{section.rows.map((row) => <Row key={row.id} row={row} held={held} failed={failed} newest={newest} task={model.task} reports={reports} />)}</ol></section>
            ))}</div></div></div>
      {pending ? <div className="sending" role="status"><span>Picking <b>{`"${pending.option}"`}</b></span>{pending.sending ? null : <button type="button" className="btn secondary" onClick={undoPick}>Undo</button>}</div> : null}
    </Panel>
  );
});

export const Reports = memo(function Reports() {
  const { size } = usePlace();
  const { data: read } = useReport(useStore(asked));
  const reports = useReports().data;
  const tasks = useTasks().data?.tasks;
  useEffect(() => {
    if (read !== undefined) accept(read);
  }, [read]);
  useEffect(() => dropPick, []);
  const head = size === "compact" ? <PageTop title="Reports" /> : <div className="lhead"><h1>Reports</h1></div>;
  const body = read
    ? <Report read={read} reports={reports} tasks={tasks} />
    : <Panel label="Reports"><p className="empty">No report yet. A workflow writes one with the <code>report</code> tool or <code>priory report put</code>.</p></Panel>;
  return <div className={listClass(size, "list")}>{head}{body}</div>;
});
