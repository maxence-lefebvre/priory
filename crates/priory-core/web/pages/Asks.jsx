// Asks (DESIGN.md, Asks panel and Ask detail): the list by saved view and facets, an ask's detail in
// the drawer or alone on a phone, and the answer sheet. The first tap on an option arms it, and a
// second within ARM_MS sends it with the revision drawn. A draft outlives the page.
import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useQuery } from "@tanstack/react-query";
import { ARM_MS, ASK_VIEWS, answeredBy, askBody, askGroups, askHeading, dismissable, optionKey, planCut, planUnread, planWait, plansFirst, press, routedMark, routedNote, runnerOf, runsCommand, seenAgo, sentNote, shownGroups, shrunkHead, snoozeShortcut, snoozed, submitted, writeNote, writeShown } from "../asks.js";
import { acceptAsk, asked, cache, loadOlder, refresh } from "../data.js";
import { deskModel, plural, sectionInView } from "../desk.js";
import { searchOf, where } from "../frame.js";
import { emptyDetail } from "../inbox.js";
import { keyOf } from "../queries.js";
import { go, history, hrefOf } from "../router.js";
import { onSignOut, post, refuse, session } from "../session.js";
import { store } from "../store.js";
import { filtered, taskLabel } from "../tasks.js";
import { age, ago, clock, date, today } from "../times.js";
import { Icon, StateWord, Tile } from "../components/Icon.jsx";
import { NotReady } from "../components/NotReady.jsx";
import { PageTop, Panel } from "../components/PhonePage.jsx";
import { SavedViews } from "../components/SavedViews.jsx";
import { Drain } from "../components/TaskActs.jsx";
import { TaskSections, sectionsOf } from "../components/TaskSections.jsx";
import { Linked, Markdown, Time } from "../components/Text.jsx";
import { usePageReads, useAsks, useDesk, useStore, useTasks } from "../hooks/useData.js";
import { useBatch } from "../hooks/useBatch.js";
import { useKeptScroll } from "../hooks/useKeptScroll.js";
import { useOlder } from "../hooks/useOlder.js";
import { useMinute } from "../hooks/useScreen.js";
import { useKeptFocus } from "../hooks/useKeptFocus.js";
import { Closes, LaneGraph, LaneKey, PlanNote, Splits, closePlanEdit, splitsOf } from "./Batch.jsx";
import { usePlace } from "./place.js";

// What the page holds beyond the reads: the answer in flight, { id, text }; the option the first tap
// armed, { id, revision, text, at, timer }; each ask's draft, note and open write form; whether the
// answer sheet is open.
const EMPTY = () => ({ pending: null, armed: null, drafts: new Map(), notices: new Map(), writing: new Set(), sheet: false });
export const answers = store(EMPTY());
const set = (part) => answers.set({ ...answers.get(), ...part });
onSignOut(() => {
  clearTimeout(answers.get().armed?.timer);
  answers.set(EMPTY());
});
const offline = () => Boolean(session.get().failed);
const NONE = [];
// The time a draw compares snoozes and leases with; useMinute draws the list again each minute.
const now = () => Date.now();

// The ask on screen, the asks its list shows, and the list it was opened from, as its detail drew them last.
let onScreen = { id: null, ask: null, listUrl: "/", picks: new Map() };

// Facets
// Each facet's key, label, an ask's values and the order the picker offers them. An ask with no
// band, such as an outside ask, shows under `none`, and so does an ask with no risk.
const facetsOf = (list) => [
  ["band", "Band", (ask) => [ask.band ?? "none"], () => [...(list?.bands || []), "none"]],
  ["risk", "Risk", (ask) => (ask.risk?.length ? ask.risk : ["none"]), () => ["irreversible", "high", "medium", "low", "red", "yellow", "green", "none"]],
  ["runner", "Runner", (ask) => [ask.runner ?? "none"], null, (name) => ((list?.asks ?? NONE).some((ask) => ask.runner === name && !runnerOf(ask)) ? `${name} (here)` : name)],
];
const picksOf = (search) => {
  const found = new URLSearchParams(search);
  return new Map(facetsOf(null).map(([key]) => [key, found.get(key)]).filter(([, value]) => value));
};
const facetQuery = (picks) => [...picks].map(([key, value]) => `&${key}=${encodeURIComponent(value)}`).join("");
const faceted = (asks, picks) => filtered(asks, picks, facetsOf(null));

/** The search of the list on screen: while an ask is open, the list keeps the one it was opened from. */
export function useListSearch() {
  const { at, href, trail } = usePlace();
  return at.ask ? searchOf(trail.listUrl) : searchOf(href);
}
const viewOf = (search) => {
  const name = new URLSearchParams(search).get("view");
  return ASK_VIEWS.includes(name) ? name : "open";
};

const Facet = memo(function Facet({ view, picks, spec: [key, label, values, order, named = (name) => name], asks }) {
  const others = new Map([...picks].filter(([other]) => other !== key));
  const picked = picks.get(key);
  if (picked) return <a className="facet" aria-current="true" href={`/asks?view=${view}${facetQuery(others)}`} aria-label={`${label}: ${picked}. Clear`}>{picked}<Icon name="x" size="s13" /></a>;
  const counts = new Map();
  for (const item of asks) for (const value of values(item)) counts.set(value, (counts.get(value) || 0) + 1);
  const offered = [...counts].sort(([a], [b]) => (order ? order().indexOf(a) - order().indexOf(b) : a.localeCompare(b)));
  return (
    <details className="facet"><summary aria-label={`Filter by ${key}`}>{label}<Icon name="chevron-down" size="s13" /></summary>
      <div className="pick">{offered.map(([name, count]) => <a key={name} href={`/asks?view=${view}${facetQuery(others)}&${key}=${encodeURIComponent(name)}`}>{named(name)}<span className="num">{count}</span></a>)}</div></details>
  );
});

// Rows
const RoutedTag = ({ ask }) => (routedMark(ask) ? <span className="tag routed">{routedMark(ask)}</span> : null);
// Only another runner's ask carries its runner. A phone row's tag keeps the name, in warn text while
// the runner is away, and says "last seen" on line 3.
export const RunnerTag = memo(function RunnerTag({ ask, full = true }) {
  const away = seenAgo(ask, now(), age);
  if (!runnerOf(ask)) return null;
  return full ? <span className="tag runner">{`${runnerOf(ask)}${away ? `, ${away}` : ""}`}</span> : <span className={`tag runner ${away ? "t-warn" : ""}`}>{runnerOf(ask)}</span>;
});
export const SentLine = memo(function SentLine({ ask }) {
  const note = sentNote(ask);
  return note ? <span className={note.refused ? "t-warn" : "t-action"}>{note.text}</span> : null;
});
const byLine = (ask) => (answeredBy(ask.answered_by) ? ` by ${answeredBy(ask.answered_by)}` : "");
const snoozedUntil = (ask) => `Snoozed until ${today(ask.snoozed_until) ? clock(ask.snoozed_until) : date(ask.snoozed_until)}`;

export const AskRow = memo(function AskRow({ ask, current }) {
  const options = ask.options || [];
  const line3 = ask.state === "answered" ? <span>Answered <b>{ask.answer}</b>{byLine(ask)}</span>
    : ask.state === "withdrawn" ? <span>{`Withdrawn${ask.withdrawn_why ? `: ${ask.withdrawn_why}` : ""}`}</span>
      : snoozed(ask, now()) ? <span><Icon name="clock" size="s15" />{snoozedUntil(ask)}</span>
        : sentNote(ask) ? <SentLine ask={ask} />
          : ask.recommendation ? <span>Recommends <b>{ask.recommendation}</b></span> : <span>{plural(options.length, "option")}</span>;
  const away = ask.state === "open" && !sentNote(ask) && seenAgo(ask, now(), age);
  return (
    <a className={`row ${ask.state !== "open" ? "past" : ""}`} href={`/asks/${ask.id}`} aria-current={current ? "true" : "false"}>
      <Tile state={ask.state === "open" ? "ask" : ask.state} />
      <span className="l1"><span className="mono">{taskLabel(ask.key)}</span><span>{ask.step}</span>{ask.command ? <span className="tag">Command</span> : null}<RunnerTag ask={ask} full={false} /><RoutedTag ask={ask} /><span className="age"><Time iso={ask.state === "answered" && ask.answered_at ? ask.answered_at : ask.since} /></span></span>
      <span className="l2">{askHeading(ask)}</span>
      <span className="l3">{away ? <span className="t-warn">{`${runnerOf(ask)} ${away}`}</span> : null}{line3}</span>
      <span className="chev"><Icon name="chevron-right" size="s15" /></span></a>
  );
});

/** The fifth column's content again under the question: CSS shows it when that column folds. Its ids take the prefix `fold-`. */
export const Fold = ({ children }) => (children ? <div className="fifth">{children}</div> : null);

const openRow = (event) => {
  // A link or control inside a row, such as its key or batch, acts on its own.
  if (!event.target.closest("a, button, details")) go(event.currentTarget.dataset.href);
};

export const AskTableRow = memo(function AskTableRow({ ask, selected }) {
  const answered = ask.state === "answered";
  const status = answered ? `${ask.answer}${byLine(ask)}` : ask.state === "withdrawn" ? ask.withdrawn_why || "" : snoozed(ask, now()) ? snoozedUntil(ask) : sentNote(ask) ? <SentLine ask={ask} /> : null;
  const folded = status ?? (ask.recommendation ? <>Recommends <b>{ask.recommendation}</b></> : "");
  return (
    <tr data-href={`/asks/${ask.id}`} aria-current={String(ask.id) === selected ? "true" : "false"} onClick={openRow}>
      <td><StateWord state={ask.state === "open" ? "ask" : ask.state} /></td><td className="mono">{taskLabel(ask.key)}</td><td>{ask.step}</td><td className="q"><span>{askHeading(ask)}</span><RunnerTag ask={ask} /><RoutedTag ask={ask} /><Fold>{folded}</Fold></td>
      <td className="recw">{status ?? ask.recommendation ?? ""}</td><td className="r"><Time iso={answered && ask.answered_at ? ask.answered_at : ask.since} /></td></tr>
  );
});

/** The last row of a list that left out older rows; scrolling to it loads them. */
export const OlderRow = memo(function OlderRow({ listing, count, wide, columns = 6 }) {
  const row = useRef(null);
  useEffect(() => {
    if (!count) return undefined;
    const observer = new IntersectionObserver(([entry]) => { if (entry.isIntersecting) loadOlder(listing); });
    observer.observe(row.current);
    return () => observer.disconnect();
  }, [listing, count]);
  if (!count) return null;
  const text = `Loading older ${listing}…`;
  return wide ? <tr className="grp" data-older={listing} ref={row}><td colSpan={columns}>{text}</td></tr> : <p className="empty" data-older={listing} ref={row}>{text}</p>;
});

export const KeyHints = () => <div className="keys"><span><kbd>J</kbd><kbd>K</kbd>move</span><span><kbd>Enter</kbd>open</span><span><kbd>Esc</kbd>close</span><span><kbd>N</kbd>add</span></div>;

const EMPTY_WORDS = { open: "No open asks. Agents ask here when they need you.", snoozed: "No snoozed asks. A snoozed ask waits here until its time.", answered: "No answered asks yet. An ask moves here once someone answers it.", all: "No asks yet. Agents ask here when they need you." };

// The asks list's saved views, facets and rows, as a table on a wide screen.
function useAsksBody(list, wide) {
  const search = useListSearch();
  const { at } = usePlace();
  const selected = at.ask;
  const picks = picksOf(search);
  const asks = faceted(list.asks, picks);
  const current = viewOf(search);
  useOlder("asks", Boolean(selected) || ["answered", "all"].includes(current));
  const { open, snoozed: asleep, answered, withdrawn } = askGroups(asks, now());
  // shortcut: older counts are not per facet, so a facet counts the loaded rows only; split `older` by facet if that misleads.
  const pending = list.older;
  const left = picks.size ? { answered: 0, withdrawn: 0 } : pending;
  const total = { open: open.length, snoozed: asleep.length, answered: answered.length + left.answered, all: asks.length + left.answered + left.withdrawn };
  const unloaded = { open: 0, snoozed: 0, answered: pending.answered, all: pending.answered + pending.withdrawn }[current];
  const views = [["open", "Open"], ["snoozed", "Snoozed"], ["answered", "Answered"], ["all", "All"]].map(([key, label]) => ({ key, label, count: total[key], on: current === key }));
  const facets = facetsOf(list).filter(([key]) => key !== "runner" || list.asks.some((ask) => ask.runner));
  const head = <><SavedViews views={views} base="/asks" facets={facetQuery(picks)} /><div className="facets" aria-label="Facets">{facets.map((spec) => <Facet key={spec[0]} view={current} picks={picks} spec={spec} asks={list.asks} />)}</div></>;
  const section = (title, rows, note, count = rows.length) => {
    if (!count) return null;
    if (wide) return [<tr key={title} className="grp"><td colSpan="6">{title}<span>{note}</span></td></tr>, ...rows.map((ask) => <AskTableRow key={ask.id} ask={ask} selected={selected} />)];
    return [<div key={title} className="sec">{title}<span>{note}</span></div>, ...rows.map((ask) => <AskRow key={ask.id} ask={ask} current={String(ask.id) === selected} />)];
  };
  const answeredToday = answered.filter((ask) => ask.answered_at && today(ask.answered_at));
  const answeredEarlier = answered.filter((ask) => !answeredToday.includes(ask));
  const shows = (group) => shownGroups(current).includes(group);
  const sections = [
    shows("open") ? section("Open", open, `${open.length}, oldest first`) : null,
    shows("snoozed") ? section("Snoozed", asleep, asleep.length) : null,
    shows("answered") ? section("Answered today", answeredToday, answeredToday.length) : null,
    shows("answered") ? section("Answered earlier", answeredEarlier, answeredEarlier.length + left.answered, answeredEarlier.length + left.answered) : null,
    shows("withdrawn") ? section("Withdrawn", withdrawn, withdrawn.length + left.withdrawn, withdrawn.length + left.withdrawn) : null,
    <OlderRow key="older" listing="asks" count={unloaded} wide={wide} />,
  ];
  const rows = wide
    ? <table className="tbl"><thead><tr><th>State</th><th>Task</th><th>Step</th><th>Question</th><th>{current === "open" ? "Recommends" : "Answer"}</th><th className="r">Age</th></tr></thead><tbody>{sections}</tbody></table>
    : sections;
  return { head, rows: total[current] || unloaded ? rows : <p className="empty">{EMPTY_WORDS[current]}</p> };
}

const useAsksList = () => useAsks(useStore(asked));

const AsksRows = memo(function AsksRows({ list, wide, phone }) {
  useMinute();
  const { head, rows } = useAsksBody(list, wide);
  const scroll = useKeptScroll(`asks ${phone} ${useListSearch()}`);
  if (phone) return <div className="scroll" ref={scroll}>{head}{rows}</div>;
  return <>{head}<div className="scroll" ref={scroll}>{rows}</div></>;
});

/** The asks beside other panels. */
export const AsksList = memo(function AsksList() {
  const result = useAsksList();
  return result.data ? <AsksListBody list={result.data} /> : <NotReady results={[result]} />;
});

const AsksListBody = memo(function AsksListBody({ list }) {
  const wide = usePlace().size === "wide";
  const part = useRef(null);
  useKeptFocus(part);
  return (
    <div className="list" role="region" aria-label="Asks" ref={part}><div className="lhead"><h1>Asks</h1></div>
      <AsksRows list={list} wide={wide} phone={false} />{wide ? <KeyHints /> : null}</div>
  );
});

/** The asks alone on a phone. */
export const AsksPage = memo(function AsksPage() {
  const result = useAsksList();
  if (!result.data) return <div className="flat"><NotReady results={[result]} /></div>;
  return <div className="flat"><PageTop title="Asks" /><Panel label="Asks"><AsksRows list={result.data} wide={false} phone /></Panel></div>;
});

// Answers: each is the twin of a CLI verb, sent with the revision the page showed.
const listedNow = () => {
  const desk = cache.getQueryData(keyOf("desk"));
  const list = cache.getQueryData(keyOf("asks", asked.get()));
  if (!desk || !list) return [];
  return faceted(deskModel({ desk, asks: list.asks, queue: {}, tasks: [] }, ago).open, onScreen.picks);
};
const shownId = () => where(hrefOf(history.location)).ask;

// An answer leaves at once: a tapped option was already confirmed by its second tap, and Send is a
// deliberate press of its own. "Sending" shows only while the request is in flight.
function answer(ask, text, written, run = false) {
  disarm();
  if (answers.get().pending) return;
  const notices = new Map(answers.get().notices);
  notices.delete(ask.id);
  // The sheet's close event comes later: a quick refusal must not find the sheet still asked for and open it again.
  set({ pending: { id: ask.id, text }, notices, sheet: false });
  send(ask, text, written, run);
}

/** The first tap on an option arms it; the second, within ARM_MS, sends it. */
function arm(armed) {
  disarm();
  set({ armed: { ...armed, timer: setTimeout(() => set({ armed: null }), ARM_MS) } });
}

export function disarm() {
  const { armed } = answers.get();
  if (!armed) return;
  clearTimeout(armed.timer);
  set({ armed: null });
}

async function send(ask, text, written, run) {
  const body = { [written ? "text" : "option"]: text, revision: ask.revision };
  if (run) body.run = true;
  const sent = await write(ask, `/api/asks/${encodeURIComponent(ask.id)}/answer`, body, text);
  set({ pending: null });
  if (sent) next(ask);
}

// True once the daemon applied the write; else the reason shows on the ask and the draft stays.
// One write at a time: a second tap on Snooze or Dismiss while the first is in flight sends nothing.
let inFlight = false;
async function write(ask, path, body, draft) {
  if (inFlight) return false;
  inFlight = true;
  let reply;
  try {
    reply = await post(path, body);
  } catch {
    reply = { status: 0, body: { error: "the connection dropped" } };
  } finally {
    inFlight = false;
  }
  const { status, body: answered } = reply;
  const { drafts, notices, writing } = answers.get();
  const [nextDrafts, nextNotices, nextWriting] = [new Map(drafts), new Map(notices), new Set(writing)];
  if (status === 200 || status === 202) {
    nextDrafts.delete(ask.id);
    nextWriting.delete(ask.id);
    nextNotices.delete(ask.id);
    set({ drafts: nextDrafts, notices: nextNotices, writing: nextWriting });
    return true;
  }
  if (status === 401) {
    refuse();
    return false;
  }
  if (draft) {
    nextDrafts.set(ask.id, draft);
    nextWriting.add(ask.id);
  }
  const fresh = answered.ask;
  if (status === 409 && fresh) await acceptAsk(fresh);
  nextNotices.set(ask.id, writeNote(status, answered, age, ask));
  set({ drafts: nextDrafts, notices: nextNotices, writing: nextWriting });
  if ((status === 409 && fresh) || status === 504 || status === 0) refresh();
  return false;
}

// After an answer, the next open ask opens, or the list once none is left.
async function next(ask) {
  const before = listedNow().map((candidate) => candidate.id);
  const { listUrl } = onScreen;
  await refresh();
  if (shownId() !== String(ask.id)) return;
  const left = listedNow().filter((candidate) => candidate.id !== ask.id);
  const following = left.find((candidate) => before.indexOf(candidate.id) > before.indexOf(ask.id)) || left[0];
  go(following ? `/asks/${encodeURIComponent(following.id)}` : listUrl);
}

async function snooze(ask, how) {
  const body = { kind: "ask", id: ask.id };
  if (how.clear) body.clear = true;
  else if (how.for) body.for = how.for;
  else {
    const at = new Date();
    at.setDate(at.getDate() + 1);
    at.setHours(how.tomorrow, 0, 0, 0);
    body.until = at.toISOString();
  }
  if (!(await write(ask, "/api/snooze", body, null))) return;
  if (body.clear) return refresh();
  next(ask);
}

async function dismiss(ask) {
  if (await write(ask, `/api/asks/${encodeURIComponent(ask.id)}/dismiss`, { revision: ask.revision }, null)) next(ask);
}

// A press of an option, by its index: the detail's, the bar's, the sheet's and the keys' own.
function pressOption(ask, index, off) {
  if (off || answers.get().pending) return;
  const pressed = press(answers.get().armed, ask, ask.options[index], Date.now());
  if (pressed.send) answer(ask, pressed.send.text, false, pressed.send.run);
  else if (pressed.arm) arm(pressed.arm);
}

// A control of the detail: nothing acts while an answer is in flight or on a disabled one.
const acting = (off, run) => (event) => {
  if (off || answers.get().pending) return;
  run(event);
};

// The draft field takes focus once Write an answer opens it. A field the layout moves, as a rotation
// does, takes back its focus and selection.
let focusDraft = false;
let movedDraft = null;
function startWriting(ask) {
  focusDraft = true;
  set({ writing: new Set(answers.get().writing).add(ask.id) });
}
function keepDraft(field) {
  if (focusDraft || (movedDraft && document.activeElement === document.body)) {
    field.focus({ preventScroll: true });
    if (movedDraft) field.setSelectionRange(...movedDraft);
  }
  focusDraft = false;
  movedDraft = null;
  return () => {
    if (document.activeElement === field) movedDraft = [field.selectionStart, field.selectionEnd, field.selectionDirection];
  };
}

const typeDraft = (ask, text) => set({ drafts: new Map(answers.get().drafts).set(ask.id, text) });

/** S snoozes the ask on screen for 1 h, 1 to 9 press its options. Never inside an input. */
export function asksKey(event) {
  const ask = onScreen.id !== null && onScreen.id === shownId() ? onScreen.ask : null;
  if (event.key === "s" || event.key === "S") {
    const body = ask && !offline() && !answers.get().pending && snoozeShortcut(ask, Date.now());
    if (body) snooze(ask, { for: body.for });
  } else if (shownId() && /^[1-9]$/.test(event.key)) {
    const index = optionKey(event);
    if (index !== null) document.querySelector(`.opt[data-answer="${index}"]`)?.click();
  } else return false;
  return true;
}

// The detail
const RAN = {
  running: "The command runs.",
  done: "The command ran: exit 0.",
  failed: "The command failed",
  uncertain: "Priory stopped while this ran. Check the target before you act on it.",
};

const Ran = memo(function Ran({ ran }) {
  if (!ran) return null;
  const text = ran.status === "failed" ? `${RAN.failed}: ${ran.error}.` : RAN[ran.status] || ran.status;
  return <p className={ran.status === "uncertain" ? "warnline" : "note"} role="status"><Icon name="terminal" size="s15" /><span>{text}</span></p>;
});

// The argv word by word, each in its own box, so a space, a quote or an empty word shows as it is.
const CommandBlock = memo(function CommandBlock({ ask, note }) {
  const { argv, target, dir } = ask.command;
  return (
    <div className="cmd"><ol className="argv" aria-label="Command, word by word">{argv.map((word, at) => <li key={at}><code>{word}</code></li>)}</ol>
      <small>In <span className="mono">{dir}</span>{` (${target}). ${note}`}</small></div>
  );
});

// An armed option: what the second tap does, the option, and the ring that drains over ARM_MS.
const ArmedLabel = ({ option, runs, at }) => <><span><small>{`Tap again to ${runs ? "run" : "send"}`}</small>{`Confirm: ${option}`}<small>{`Cancels in ${ARM_MS / 1000} s`}</small></span><Drain at={at} /></>;

const armedOn = (armed, ask) => (armed?.id === ask.id && armed.revision === ask.revision ? armed : null);

// One button per option, as the detail and the answer sheet draw them. An armed command option also shows its command.
const OptionList = memo(function OptionList({ ask, wide, locked, bound }) {
  const { pending: held, armed: armedAny } = useStore(answers);
  const pending = held?.id === ask.id ? held : null;
  const armed = armedOn(armedAny, ask);
  return (ask.options || []).map((option, i) => {
    const recommended = option === ask.recommendation;
    const runs = runsCommand(ask, option);
    const chosen = pending?.text === option;
    const off = locked || (pending && !chosen);
    const notes = [recommended && "Recommended", runs && "Runs a command"].filter(Boolean).join(". ");
    const on = armed?.text === option;
    const button = (
      <button key={i} type="button" className={`opt ${recommended ? "rec-opt" : ""} ${chosen ? "chosen" : ""} ${on ? "armed" : ""}`} data-option={i} data-answer={i} aria-disabled={off || pending ? "true" : "false"} aria-describedby={locked ? "answer-locked" : undefined} onClick={() => pressOption(ask, i, Boolean(locked))}>
        {chosen ? <Icon name="check" size="s20" /> : null}{on ? <ArmedLabel option={option} runs={runs} at={armed.at} /> : <span>{option}{notes ? <small>{notes}</small> : null}</span>}{wide ? <kbd>{i + 1}</kbd> : null}</button>
    );
    return on && runs ? [button, <CommandBlock key={`${i}cmd`} ask={ask} note={bound} />] : button;
  });
});

const CutNote = memo(function CutNote({ ask, task }) {
  const cut = planCut(ask, task);
  return cut ? <p className="note warn"><Icon name="file-text" size="s15" /><span>{cut}</span></p> : null;
});

// Why an ask's options are disabled: offline, or an ask whose plan is not read yet for this
// revision, so no answer approves a plan the page did not show.
function lockOf(ask, task, batch) {
  if (offline()) return "offline";
  if (runnerOf(ask)) return null;
  if (planUnread(ask, task)) return "plan";
  return ask.state === "open" && ask.key?.startsWith("batch:") && !batch ? "plan" : null;
}

const LockedNote = memo(function LockedNote({ ask, task, locked, error, id }) {
  if (locked === "offline") return <p className="note" id={id}><Icon name="unplug" size="s15" /><span>Reconnect to answer.</span></p>;
  if (locked) return <p className="note" id={id}><Icon name="layers" size="s15" /><span>{planWait(ask, error)}</span></p>;
  return <CutNote ask={ask} task={task} />;
});

// History: every revision and refused answer, newest first; an ask listed before Priory kept them shows its current revision.
const AskHistory = memo(function AskHistory({ ask }) {
  const dated = [
    ...(ask.revisions || [{ revision: ask.revision, at: ask.since, answer: ask.answer, answered_at: ask.answered_at }]).flatMap((revision) => [
      [revision.at, <li key={`r${revision.revision}`}><Tile state="ask" size="sm" /><span>Asked revision <b>{revision.revision}</b>{revision.question ? `: ${revision.question}` : ""}</span><Time iso={revision.at} /></li>],
      ...(revision.answer == null ? [] : [[revision.answered_at || revision.at, <li key={`a${revision.revision}`}><Tile state="answered" size="sm" /><span>Answered <b>{revision.answer}</b>{ask.state === "answered" && revision.revision === ask.revision ? byLine(ask) : ""}</span><Time iso={revision.answered_at || revision.at} /></li>]]),
    ]),
    ...(ask.refused || []).map((refusal, at) => [refusal.at, <li key={`x${at}`}><Tile state="refused" size="sm" /><span className="t-warn"><b>Refused:</b>{` ${refusal.answer}, sent for revision ${refusal.revision} from ${refusal.by}: ${refusal.why}`}</span><Time iso={refusal.at} /></li>]),
    ...submitted(ask).map((sent, at) => [sent.at, <li key={`s${at}`}><Tile state={sent.tile} size="sm" /><span className={sent.tile === "refused" ? "t-warn" : ""}>{sent.text}</span><Time iso={sent.at} /></li>]),
  // Built oldest first; reversed, entries of the same second stay newest first through the stable sort.
  ].reverse().sort(([a], [b]) => b.localeCompare(a));
  const items = dated.map(([, item]) => item);
  if (ask.state === "withdrawn") items.unshift(<li key="w"><Tile state="withdrawn" size="sm" /><span>{`Withdrawn${ask.withdrawn_why ? `: ${ask.withdrawn_why}` : ""}`}</span><span /></li>);
  return <section className="sect" id="history"><div className="sh"><Icon name="history" size="s15" />Answer history</div><ol className="hist">{items}</ol></section>;
});

const Why = memo(function Why({ ask }) {
  const body = askBody(ask);
  return (
    <section className="sect" id="why"><div className="sh"><Icon name="message-text" size="s15" />What the agent asks<span className="meta">{ask.run ? <>run <span className="mono">{ask.run}</span></> : null}</span></div>
      {body ? <p className="asked"><Linked text={body} repo={ask.repo} /></p> : null}
      {ask.text || !body ? <Markdown text={ask.text || ask.question} repo={ask.repo} /> : null}
      {ask.tradeoff ? <p className="fact"><Icon name="scale" size="s15" /><span><b>Trade-off.</b>{` ${ask.tradeoff}`}</span></p> : null}
      {ask.what_waits ? <p className="fact"><Icon name="hourglass" size="s15" /><span><b>What waits.</b>{` ${ask.what_waits}`}</span></p> : null}</section>
  );
});

const Command = memo(function Command({ ask, owner }) {
  if (!ask.command) return null;
  if (typeof ask.command === "string") {
    return (
      <section className="sect" id="command"><div className="sh"><Icon name="terminal" size="s15" />Command the asker attached</div>
        <div className="cmd"><span className="mono">{`$ ${ask.command}`}</span><small>Priory stores this command and never runs it: the ask came from outside Priory.</small></div></section>
    );
  }
  return (
    <section className="sect" id="command"><div className="sh"><Icon name="terminal" size="s15" />Command an answer runs</div>
      <CommandBlock ask={ask} note={`${owner ? `Runs on ${owner} when` : "Runs when"} answered ${ask.command.runs.join(" or ")}. Bound to revision ${ask.revision}. Runs at most once.`} /></section>
  );
});

// The section chip of the section in view.
// It lights again after each draw, which may move the sections.
function useLit(scroll, count) {
  const [lit, setLit] = useState(null);
  useEffect(() => {
    const element = scroll.current;
    if (!element || !count) return undefined;
    const light = () => {
      const sections = [...element.querySelectorAll(".sect")];
      const top = element.getBoundingClientRect().top - element.scrollTop;
      const tops = sections.map((section) => section.getBoundingClientRect().top - top);
      // A chip lands its section under the chip bar, at the section's scroll-margin-top.
      const bar = sections.length ? parseFloat(getComputedStyle(sections[0]).scrollMarginTop) || 0 : 0;
      const at = sectionInView({ tops, scrollTop: element.scrollTop, height: element.clientHeight, scrollHeight: element.scrollHeight, bar });
      const chip = element.querySelector(`.jump a[href="#${sections[at]?.id}"]`);
      if (!chip) return;
      setLit(sections[at].id);
      // On a phone the chips scroll sideways: keep the lit one in view.
      const strip = chip.parentElement;
      if (strip.scrollWidth > strip.clientWidth) strip.scrollLeft = chip.offsetLeft - (strip.clientWidth - chip.offsetWidth) / 2;
    };
    element.addEventListener("scroll", light, { passive: true });
    light();
    return () => element.removeEventListener("scroll", light);
  });
  return lit;
}

// The answer bar shows once the options scroll away; only options scrolled past shrink the header.
function showBar(part, { bar: shown, shrunk }) {
  const bar = part.current?.querySelector(".abar");
  if (bar) bar.hidden = !shown;
  part.current?.querySelector(".ahead:has(.aq)")?.classList.toggle("shrunk", shrunk);
}
function useBar(part, scroll, ready) {
  const state = useRef({ bar: false, shrunk: false });
  useEffect(() => {
    const options = scroll.current?.querySelector(".opts");
    if (!options) return undefined;
    const observer = new IntersectionObserver(([entry]) => {
      state.current = { bar: !entry.isIntersecting, shrunk: !entry.isIntersecting && entry.boundingClientRect.bottom <= (entry.rootBounds?.top ?? 0) };
      showBar(part, state.current);
    }, { root: scroll.current });
    observer.observe(options);
    return () => observer.disconnect();
  }, [part, scroll, ready]);
  // A bar drawn again keeps the place the options left it in.
  useLayoutEffect(() => showBar(part, state.current));
}

const Nav = ({ to, label, name }) => (to
  ? <a className="iconbtn" href={`/asks/${to.id}`} aria-label={label}><Icon name={name} size="s20" /></a>
  : <span className="iconbtn" aria-disabled="true" aria-label={label}><Icon name={name} size="s20" /></span>);

// The answer sheet (DESIGN.md, Answer while reading): every option of the ask on screen, opened by the bar's "N options".
const sheet = () => document.getElementById("answer");
const AnswerSheet = memo(function AnswerSheet({ ask, task, locked, bound }) {
  const { sheet: open, pending, notices } = useStore(answers);
  const shown = open && ask.state === "open" && !pending;
  const sent = useRef(false);
  useEffect(() => {
    const dialog = sheet();
    const closed = () => set({ sheet: false });
    const outside = (event) => { if (event.target === dialog) dialog.close(); };
    dialog.addEventListener("close", closed);
    dialog.addEventListener("click", outside);
    return () => {
      dialog.removeEventListener("close", closed);
      dialog.removeEventListener("click", outside);
      // The close listener is gone, so clear the store here, or the next ask opens the sheet unasked.
      set({ sheet: false });
      if (dialog.open) dialog.close();
    };
  }, []);
  // Closes itself once the ask is answered, or its answer is sending.
  useLayoutEffect(() => {
    const dialog = sheet();
    if (shown && !dialog.open) dialog.showModal();
    if (!shown && dialog.open) {
      sent.current = Boolean(pending);
      dialog.close();
    }
    // A refused answer leaves the ask open, and focus goes back to the bar's options button, which
    // the bar draws again after this effect.
    if (!pending && sent.current) {
      sent.current = false;
      if (notices.has(ask.id)) setTimeout(() => document.querySelector(".abar .btn.secondary")?.focus());
    }
  }, [shown, pending, notices, ask.id]);
  if (!shown) return null;
  return createPortal(
    <>
      <div className="shead"><h2 id="answer-title">{`Answer ask #${ask.id}`}<small>{`revision ${ask.revision}`}</small></h2><button type="button" className="iconbtn" aria-label="Close" onClick={() => sheet().close()}><Icon name="x" size="s20" /></button></div>
      <div className="opts" role="group" aria-label="Options"><OptionList ask={ask} wide={false} locked={locked} bound={bound} /></div><CutNote ask={ask} task={task} />
    </>,
    sheet(),
  );
});

/** The go ask's batch, once the batch names this ask as its go, read for this revision of the ask. */
function useGoBatch(ask) {
  // Another runner's batch lives on that runner; this server cannot read its plan.
  const which = ask?.key?.startsWith("batch:") && !runnerOf(ask) ? ask.key : null;
  const result = useBatch(which, ask?.revision ?? null);
  const enabled = Boolean(which);
  const batch = enabled && result.data?.ask === ask.id ? result.data : null;
  return { batch, read: enabled ? result.data ?? null : null, error: enabled ? result.error : null };
}

// The ask's task as /api/tasks/{id} serves it, read again with each refresh.
// Like a page's own read, it waits while the session failed: the listings' reads heal it.
function useAskTask(ask) {
  const id = ask?.task && !runnerOf(ask) ? ask.task : null;
  const reads = usePageReads();
  const result = useQuery({ queryKey: ["task", id], enabled: reads && id !== null });
  return id ? result : { data: undefined, error: null };
}

const AskBody = memo(function AskBody({ ask, id, wide, compact, list, listUrl, listed, part }) {
  useMinute();
  // Offline locks the options.
  useStore(session);
  const { pending: held, armed: armedAny, notices, writing, drafts } = useStore(answers);
  const tasks = useTasks(useStore(asked)).data?.tasks;
  const { data: task, error: taskError } = useAskTask(ask);
  const { batch, read: batchRead, error: batchError } = useGoBatch(ask);
  const scroll = useKeptScroll(`ask ${wide} ${compact} ${id}`);
  const pending = held?.id === ask.id ? held : null;
  const armed = armedOn(armedAny, ask);
  const owner = runnerOf(ask);
  const locked = lockOf(ask, task, batchRead);
  const planError = (ask.approves_plan ? taskError : batchError)?.message;
  const bound = `${owner ? `Runs on ${owner}. ` : ""}Bound to revision ${ask.revision}.`;
  const options = ask.options || [];
  const at = listed.findIndex((candidate) => String(candidate.id) === id);
  const back = [listUrl, list === "asks" ? "Asks" : "Desk"];
  const shrunk = shrunkHead(ask);
  const notice = notices.get(ask.id);
  const away = seenAgo(ask, now(), age);
  const sent = sentNote(ask);
  const editable = Boolean(batch) && ask.state === "open" && batch.state === "asked" && !locked;
  // A new revision is a new question: a tap armed on the old one must not run it.
  useEffect(() => {
    const { armed: now } = answers.get();
    if (now && (now.id !== ask.id || now.revision !== ask.revision)) disarm();
  }, [ask.id, ask.revision]);
  const sectionsRead = (task ? sectionsOf(task, tasks) : []).filter(([key]) => key !== "plan" || (!batch && !planUnread(ask, task)));
  const lead = batch || !plansFirst(ask) ? [] : sectionsRead.filter(([key]) => key === "plan");
  const read = sectionsRead.filter((section) => !lead.includes(section));
  const planned = batch ? [["batchplan", "Plan"], ...(batch.closes?.length ? [["closes", "Closes"]] : []), ...(splitsOf(batch).length ? [["splits", "Splits"]] : [])] : [];
  const chips = [...planned, ...lead, ["why", "Why"], ...read, ["history", "History"], ...(ask.command ? [["command", "Command"]] : [])];
  const lit = useLit(scroll, chips.length);
  // A resize moves the options in or out of the scroll.
  useBar(part, scroll, `${wide} ${options.length}`);
  const header = (
    <div className="ahead">
      {wide
        ? <span className="ctx"><Tile state="ask" size="sm" /><span>ask <span className="mono">{`#${ask.id}`}</span></span><span>{`revision ${ask.revision}`}</span><span><Time iso={ask.since} /></span></span>
        : <><a className="back" href={back[0]} aria-label={back[1]}><Icon name="chevron-left" size="s22" /><span>{back[1]}</span></a><span className="aq" aria-hidden="true"><b>{shrunk.question}</b><small>{shrunk.place}</small></span></>}
      {at >= 0 ? <span className="pos num">{at + 1}<span>{`/${listed.length}`}</span></span> : <span className="sp" />}
      <Nav to={listed[at - 1]} label="Previous ask" name="chevron-up" /><Nav to={listed[at + 1]} label="Next ask" name="chevron-down" />
      {wide ? <a className="iconbtn" href={back[0]} aria-label="Close"><Icon name="x" size="s20" /></a> : null}</div>
  );
  const shown = writeShown(writing.has(ask.id), locked, Boolean(pending));
  const busy = pending ? "true" : "false";
  const submit = (event) => {
    event.preventDefault();
    const text = (answers.get().drafts.get(ask.id) || "").trim();
    if (text && writeShown(answers.get().writing.has(ask.id), lockOf(ask, task, batchRead), Boolean(answers.get().pending)) === "form") answer(ask, text, true);
  };
  const writeForm = shown === "form"
    ? <form className="writeform" onSubmit={submit}><label className="sr" htmlFor="draft">Your answer</label><textarea id="draft" name="text" rows="3" ref={keepDraft} value={drafts.get(ask.id) || ""} onChange={(event) => typeDraft(ask, event.target.value)} /><button type="submit" className="btn primary" aria-disabled={pending ? "true" : undefined}>Send</button></form>
    : <button type="button" className="ghost write" aria-disabled={shown === "disabled" ? "true" : "false"} onClick={acting(shown === "disabled", () => startWriting(ask))}><Icon name="pencil" size="s15" />Write an answer</button>;
  const snoozeButtons = !ask.task || owner || locked === "offline" ? null
    : ask.snoozed_until
      ? <button type="button" className="ghost" aria-disabled={busy} onClick={acting(false, () => snooze(ask, { clear: true }))}>Unsnooze</button>
      : <><button type="button" className="ghost" aria-disabled={busy} onClick={acting(false, () => snooze(ask, { for: "1h" }))}>Snooze 1 h</button><button type="button" className="ghost" aria-disabled={busy} onClick={acting(false, () => snooze(ask, { tomorrow: 9 }))}>Snooze until tomorrow 09:00</button></>;
  const more = (
    <div className="more">{editable ? <a className="ghost" href="#batchplan"><Icon name="pencil" size="s15" />Edit the plan</a> : null}{writeForm}
      {locked || !dismissable(ask) ? null : <button type="button" className="ghost" aria-disabled={busy} onClick={acting(false, () => dismiss(ask))}>Dismiss</button>}{snoozeButtons}</div>
  );
  // A closed ask still says why its runner refused an answer sent to it.
  const sentShown = !sent || notice || (ask.state !== "open" && !sent.refused) ? null : sent.refused
    ? <p className="warnline" role="status">{`${sent.text}.`}</p>
    : <p className="routednote" role="status"><Icon name="corner-down-right" size="s15" /><span>{`${sent.text}. It applies when ${owner} reads the log.${away ? ` ${owner} was ${away}.` : ""}`}</span></p>;
  const noticeLine = notice ? <p className="warnline" role="status">{notice}</p> : null;
  const decided = ask.state !== "open"
    ? <>{noticeLine}{sentShown}<div className="rec">{ask.state === "answered" ? <span><b>{`Answered: ${ask.answer}.`}</b>{` ${ask.answered_at ? ago(ask.answered_at) : ""}${byLine(ask) ? ` By ${answeredBy(ask.answered_by)}.` : ""}`}</span> : <span><b>{`Withdrawn${ask.withdrawn_why ? `: ${ask.withdrawn_why}` : ""}.`}</b></span>}</div><Ran ran={ask.ran} /></>
    : <>{noticeLine}{sentShown}{routedNote(ask) ? <p className="routednote"><Icon name="corner-down-right" size="s15" />{routedNote(ask)}</p> : null}{ask.recommendation ? <div className="rec"><span><b>{`Recommended: ${ask.recommendation}.`}</b></span>
        {ask.next_action ? <span className="then"><Icon name="corner-down-right" size="s15" /><span>{ask.next_action}</span></span> : null}</div> : null}
      <div className="opts" role="group" aria-label="Options"><OptionList ask={ask} wide={wide} locked={locked} bound={bound} /></div><LockedNote ask={ask} task={task} locked={locked} error={planError} id="answer-locked" />{more}
      {owner ? <p className="note"><Icon name="corner-down-right" size="s15" /><span>{`This ask belongs to ${owner}. Your answer goes there through the log, and ${owner} applies it when it reads it. Snooze stays on ${owner}.`}</span></p> : null}</>;
  const top = (
    <div className="ask">
      <a className="ctx" href={ask.task && !owner ? `/tasks/${ask.task}` : "/asks"} title={ask.task && !owner ? `task ${ask.task}` : ""}>{wide ? null : <Tile state="ask" size="sm" />}<span className="mono">{taskLabel(ask.key)}</span><span>{ask.step}</span><span>{ask.lane || ""}</span>{ask.command ? <span className="tag">Command</span> : null}<RunnerTag ask={ask} />{wide ? null : <><span><Time iso={ask.since} /></span><span>{`revision ${ask.revision}`}</span></>}</a>
      {ask.risk_estimate ? <p className="note"><Icon name="lock" size="s15" /><span>{ask.risk_estimate.line}</span></p> : null}
      {compact ? <h1 className="question"><Linked text={askHeading(ask)} repo={ask.repo} /></h1> : <h2 className="question"><Linked text={askHeading(ask)} repo={ask.repo} /></h2>}{decided}</div>
  );
  // A go ask reads its batch's plan first; the planning task's own plan result says the same.
  const plan = batch ? (
    <>
      <section className="sect" id="batchplan"><div className="sh"><Icon name="layers" size="s15" />{`Plan, ${plural(batch.entries.length, "task")} in start order`}<span className="meta">{`rev ${batch.plan_revision}`}</span></div><LaneKey /><LaneGraph batch={batch} editable={editable} /><PlanNote batch={batch} /></section>
      <Closes batch={batch} /><Splits batch={batch} />
    </>
  ) : null;
  const jump = <nav className="jump" aria-label="Sections">{chips.map(([key, label]) => <a key={key} href={`#${key}`} aria-current={(lit ?? chips[0][0]) === key ? "true" : "false"}>{label}</a>)}</nav>;
  // A task read again that failed keeps its last read, and says so (DESIGN.md, Error). Offline, the strip says it.
  const unread = task && taskError && !offline() ? <p className="note" role="status">{`Could not read task ${ask.task} again: ${taskError.message}. This is its last read.`}</p> : null;
  const sections = (
    <>
      {jump}{plan}{lead.length ? <TaskSections task={task} tasks={tasks} only={["plan"]} /> : null}<Why ask={ask} />{unread}
      {task ? <TaskSections task={task} tasks={tasks} only={read.map(([key]) => key)} /> : null}<AskHistory ask={ask} /><Command ask={ask} owner={owner} />
    </>
  );
  const sending = pending ? <div className="sending" role="status"><span>Sending <b>{`"${pending.text}"`}</b></span></div> : null;
  if (wide) return <><div className="pinned">{header}{top}{sending}</div><div className="scroll" data-sections ref={scroll}>{sections}</div></>;
  const recommended = options.find((option) => option === ask.recommendation) || options[0];
  const barArmed = armed !== null && armed.text === recommended;
  const runs = runsCommand(ask, recommended);
  const bar = !pending && ask.state === "open" && options.length ? (
    <div className="abar" hidden>{barArmed && runs ? <CommandBlock ask={ask} note={bound} /> : null}<button type="button" className="btn secondary" onClick={acting(false, () => set({ sheet: true }))}><Icon name="chevron-up" size="s15" />{plural(options.length, "option")}</button>
      <button type="button" id="bar-answer" className={`btn primary ${barArmed ? "armed" : ""}`} data-answer={options.indexOf(recommended)} aria-disabled={locked ? "true" : "false"} onClick={() => pressOption(ask, options.indexOf(recommended), Boolean(locked))}>{barArmed
        ? <ArmedLabel option={recommended} runs={runs} at={armed.at} />
        : <><span>{recommended}</span><small>{[recommended === ask.recommendation ? "Recommended" : "First option", runs && "Runs a command"].filter(Boolean).join(". ")}</small></>}</button>
      <LockedNote ask={ask} task={task} locked={locked} error={planError} /></div>
  ) : null;
  return (
    <>
      {header}<div className="scroll" data-sections ref={scroll}>{top}{sections}</div>{bar}{sending}
      <AnswerSheet ask={ask} task={task} locked={locked} bound={bound} />
    </>
  );
});

// The open asks the list shows, so previous, next and the ask after an answer stay in its facets.
function useListed(picks) {
  const desk = useDesk().data;
  const list = useAsksList().data;
  if (!desk || !list) return NONE;
  return faceted(deskModel({ desk, asks: list.asks, queue: {}, tasks: [] }, ago).open, picks);
}

/** One ask, in the drawer or alone on a phone. `part` holds the element it draws in. */
function AskDetail({ id, wide, compact, part }) {
  const { trail } = usePlace();
  const asksResult = useAsksList();
  const picks = picksOf(searchOf(trail.listUrl));
  const listed = useListed(picks);
  const list = asksResult.data;
  const ask = list?.asks.find((candidate) => String(candidate.id) === id) ?? null;
  const older = list ? list.older.answered + list.older.withdrawn : 0;
  useOlder("tasks", false);
  useEffect(() => {
    onScreen = { id, ask, listUrl: trail.listUrl, picks };
  });
  useEffect(() => {
    if (list && !ask && older) loadOlder("asks");
  }, [list, ask, older]);
  // Leaving the ask disarms its option and closes the plan's menus.
  useEffect(() => () => {
    disarm();
    closePlanEdit();
    if (onScreen.id === id) onScreen = { ...onScreen, id: null, ask: null };
  }, [id]);
  const heading = compact ? <h1 className="sr">{`Ask #${id}`}</h1> : null;
  if (!ask) return <>{heading}<p className="empty">{older ? `Loading ask ${id}…` : <>{`No ask ${id} here. It may belong to another factory. Check `}<code>priory asks --all</code>.</>}</p></>;
  return <>{heading}<AskBody ask={ask} id={id} wide={wide} compact={compact} list={trail.list} listUrl={trail.listUrl} listed={listed} part={part} /></>;
}

// Whether the ask's listings are read; else the part says why, when no part before it does.
function useAskReady() {
  const asksResult = useAsksList();
  const tasksResult = useTasks(useStore(asked));
  return { ready: Boolean(asksResult.data && tasksResult.data), results: [asksResult, tasksResult], listsAsks: Boolean(asksResult.data) };
}

/** An ask alone on a phone. */
export const AskPage = memo(function AskPage({ id }) {
  const part = useRef(null);
  const { ready, results } = useAskReady();
  return <div className="flat" ref={part}>{ready ? <AskDetail id={id} wide={false} compact part={part} /> : <NotReady results={results} />}</div>;
});

/** The drawer beside a list: the ask the address names, or what to select. */
export const AskDrawer = memo(function AskDrawer({ id, panels }) {
  // With no ask it needs nothing.
  return id ? <AskInDrawer id={id} /> : <aside className="drawer" aria-label="Ask"><p className="empty">{emptyDetail(panels)}</p></aside>;
});

const AskInDrawer = memo(function AskInDrawer({ id }) {
  const part = useRef(null);
  const { size, trail } = usePlace();
  const { ready, results, listsAsks } = useAskReady();
  // The list beside it speaks first: the Desk needs the tasks too, the Asks list only the asks.
  if (!ready) return <NotReady results={results} speaks={listsAsks && trail.list !== "desk"} />;
  return <aside className="drawer" aria-label="Ask" ref={part}><AskDetail id={id} wide={size === "wide"} compact={false} part={part} /></aside>;
});
