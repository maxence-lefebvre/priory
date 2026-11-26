// The task page's terminal pane (DESIGN.md, Terminal pane): the run's screen, polled while shown and
// never kept on this device, its Say line, a held keyboard's bar, and the sheet that takes or gives it.
import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal, flushSync } from "react-dom";
import { plural } from "../desk.js";
import { STATES } from "../icons.js";
import { EARLIER_CAP, KEYS, keyboardSession, sayLine, screenRows, screenWatch } from "../terminal.js";
import { api, onSignOut, post, session, signOut } from "../session.js";
import { store } from "../store.js";
import { clock } from "../times.js";
import { useStore } from "../hooks/useData.js";
import { Icon, Tile } from "./Icon.jsx";

const REFUSED = "The daemon refused this token. Paste the current one.";
// Moves whenever the screen, the keyboard or the pane's own state changed, so the pane draws again.
const painted = store(0);
const paint = () => painted.set(painted.get() + 1);
const paintNow = () => flushSync(paint);

export const watch = screenWatch({
  read: (run, have, history) => api(`/api/runs/${run}/screen?${new URLSearchParams({ history, ...(have ? { have } : {}) })}`).catch((error) => {
    if (error.status === 401) signOut(REFUSED);
    throw error;
  }),
  changed: paint,
});
// This page's hold on the pane's keyboard (terminal.js). Its writes keep going while the page unloads.
export const keyboard = keyboardSession({
  post: (path, body) => fetch(path, {
    method: "POST",
    keepalive: true,
    headers: { Authorization: `Bearer ${session.get().token}`, "Content-Type": "application/json" },
    body: JSON.stringify(body),
  }).then(async (response) => ({ status: response.status, body: await response.json().catch(() => ({})) })),
  signOut: () => signOut(REFUSED),
  changed: paint,
  format: (iso) => clock(iso),
});
// While the token still works: giving the keyboard back needs it.
onSignOut(() => {
  watch.stop();
  keyboard.leave();
});

// The pane's own state for run `run`. `left` is its sideways scroll, kept while the page draws again.
// `typing` opens the Say line, `line` holds its text, `said` its last note and whether it was said.
// `menu` opens the header's More, `keys` holds the text the keys bar pastes, and `sheet` the open
// keyboard sheet, { kind, run, tail }.
const RESET = { left: 0, typing: false, line: "", said: null, saying: false, menu: false, keys: "" };
const terminal = { fit: false, copied: null, run: null, sheet: null, ...RESET };

export const screenText = (screen) => screenRows(screen.lines).map((parts) => parts.map((part) => part.text).join("")).join("\n").trimEnd();

/** The live run, else the run whose last screen this page still holds. */
export function terminalRun(task) {
  const runs = task.runs || [];
  const kept = watch.state();
  return runs.findLast((run) => !run.ended) || runs.find((run) => run.id === kept.run && kept.screen) || null;
}

const focus = (selector) => document.querySelector(selector)?.focus();
// The note reads the clock as the pane draws: each poll that fails draws it again.
const secondsSince = (at) => Math.round((Date.now() - at) / 1000);

function copy(text, what) {
  navigator.clipboard?.writeText(text).then(() => {
    terminal.copied = what;
    paint();
    setTimeout(() => {
      terminal.copied = null;
      paint();
    }, 2000);
  }, () => {});
}

// Sends one input; a text leaves the field once it was sent, unless the person typed on.
async function sendInput(input) {
  const sent = await keyboard.send(input);
  if (sent && input.text !== undefined && terminal.keys === input.text) terminal.keys = "";
  if (sent) watch.revision();
  paint();
}

// Sends the Say line; a refusal keeps its text, so a tap sends it again.
async function say(task) {
  const [text, run] = [terminal.line, terminal.run];
  if (terminal.saying || !text.trim()) return;
  terminal.saying = true;
  paint();
  const said = await sayLine(post, task, text);
  if (said.signOut) return signOut(REFUSED);
  // The pane moved to another run meanwhile: its line and note are not this one's.
  if (terminal.run !== run) return;
  terminal.saying = false;
  // Text typed while the line was in flight stays.
  if (said.said && terminal.line === text) terminal.line = "";
  terminal.said = said;
  paint();
}

function toggleTyping() {
  Object.assign(terminal, { typing: !terminal.typing, said: null });
  paintNow();
  if (terminal.typing) focus("#term-line");
}

function askKeyboard(kind) {
  terminal.menu = false;
  // The sheet acts on the run it names, never on one the pane switched to under it.
  const tail = (watch.state().screen?.lines || []).map((line) => line.replace(/\x1b\[[0-9;:]*m/g, "")).filter((line) => line.trim()).slice(-2).join("\n");
  terminal.sheet = { kind, run: keyboard.state().run, tail };
  paint();
}

const sheet = () => document.getElementById("keyboard");
// Closing the sheet hands focus back to the pane's More.
function sheetClosed() {
  terminal.sheet = null;
  paint();
  focus("#app [data-action='term-menu']");
}

async function keyboardAct(kind) {
  const { run } = keyboard.state();
  const asked = terminal.sheet?.run;
  sheet().close();
  if (run !== asked) return;
  if (kind === "take") {
    terminal.typing = false;
    await keyboard.take(run);
    paintNow();
    return focus("#term-keys");
  }
  await keyboard.giveBack();
  watch.revision();
}

function escape(event) {
  if (event.key !== "Escape") return;
  if (terminal.menu) {
    terminal.menu = false;
    paintNow();
    focus('[data-action="term-menu"]');
  } else if (event.target.id === "term-line") {
    Object.assign(terminal, { typing: false, said: null });
    paintNow();
    focus('[data-action="term-type"]');
  } else return;
  event.stopPropagation();
}

const ScreenPart = ({ part }) => {
  const inner = part.hidden ? <span className="hid">{part.text}</span> : part.cls ? <span className={part.cls}>{part.text}</span> : part.text;
  return part.cursor ? <span className="cur">{inner}</span> : inner;
};
// One row of the screen. A row that did not change keeps its element, so a selection elsewhere stays.
const Row = memo(function Row({ parts, earlier, fold }) {
  const shown = parts.map((part, at) => <ScreenPart key={at} part={part} />);
  return <span>{earlier ? <span className="earlier">{shown}</span> : shown}{"\n"}{fold ? <span className="fold" aria-hidden="true" /> : null}</span>;
}, (before, after) => before.sig === after.sig);

// The rows of the screen. The cursor shows only on a live run's screen with no earlier rows above it.
function Rows({ screen, end }) {
  if (!screen) return null;
  const earlier = screen.earlier || 0;
  return screenRows(screen.lines, earlier || end ? null : screen.cursor).map((parts, at) => {
    const [above, fold] = [at < earlier, at === earlier - 1];
    return <Row key={at} sig={JSON.stringify([parts, above, fold])} parts={parts} earlier={above} fold={fold} />;
  });
}

// The bar of a held keyboard: whose and until when, a text pasted without Enter, and one button a key.
function KeysBar({ held, keys: { note, returning, pasting } }) {
  const paste = (event) => {
    event.preventDefault();
    if (terminal.keys) sendInput({ text: terminal.keys });
  };
  return (
    <div className="keys-bar"><p className="who"><Icon name="keyboard" size="s15" /><span><b>Your keys</b>{` until ${clock(held.ends)}`}</span><span className="sp" /><button type="button" className="give" data-action="term-give-ask" aria-disabled={returning ? "true" : undefined} onClick={() => askKeyboard("give")}>{returning ? "Giving back…" : "Give back"}</button></p>
      <form data-action="term-paste" onSubmit={paste}><span className="pr" aria-hidden="true">›</span><label className="sr" htmlFor="term-keys">Text to paste, without Enter</label>
        <input id="term-keys" autoComplete="off" autoCapitalize="off" spellCheck="false" placeholder="Text, sent without Enter" value={terminal.keys} onChange={(event) => { terminal.keys = event.target.value; paint(); }} /><button type="submit" className="send" aria-disabled={pasting || returning ? "true" : undefined}>Send</button></form>
      <div className="keys-row" role="group" aria-label="Keys">{KEYS.map((key, at) => (key
        ? <button key={at} type="button" data-action="term-key" data-key={key[0]} aria-label={key[0]} className={key[2] ? "hot" : undefined} onClick={() => sendInput({ key: key[0] })}>{key[1]}</button>
        : <span key={at} className="gap" aria-hidden="true" />))}</div>
      {note ? <p className="tn" role="status"><Icon name="check" size="s15" /><span>{note}</span></p> : null}</div>
  );
}

// The confirm sheet before a person takes the keyboard, or gives it back.
function KeyboardSheet({ kind, run, tail }) {
  useLayoutEffect(() => {
    const dialog = sheet();
    if (!dialog.open) dialog.showModal();
    dialog.querySelector(".acts .btn:last-child")?.focus();
  }, [kind, run]);
  const close = <button type="button" className="iconbtn" data-action="keyboard-close" aria-label="Close" onClick={() => sheet().close()}><Icon name="x" size="s20" /></button>;
  return createPortal(kind === "take" ? (
    <><div className="shead"><h2 id="keyboard-title">{`Take the keyboard of run ${run}?`}</h2>{close}</div>
      <p className="warnline"><Icon name="shield" size="s15" /><span>Your keys reach the agent and its shell directly. Priory&apos;s guard does not check them.</span></p>
      <ul className="facts"><li><Icon name="clock" size="s15" /><span>For 15 minutes, or until you give it back.</span></li>
        <li><Icon name="pause" size="s15" /><span>The agent&apos;s queued prompts and your Says wait until then.</span></li>
        <li><Icon name="pencil" size="s15" /><span>Priory records each key and the length of each text, never the text.</span></li></ul>
      <div className="acts"><button type="button" className="btn secondary" data-action="keyboard-close" onClick={() => sheet().close()}>Keep watching</button><button type="button" className="btn danger" data-action="term-take" onClick={() => keyboardAct("take")}><Icon name="keyboard" size="s15" />Take the keyboard</button></div></>
  ) : (
    <><div className="shead"><h2 id="keyboard-title">Give the keyboard back?</h2>{close}</div>
      <p className="line">Anything you typed and did not submit stays in the agent&apos;s input box, and its next prompt joins it. Clear it with ⌃C or submit it with ⏎ first.</p>
      {tail ? <pre className="peek">{tail}</pre> : null}
      <div className="acts"><button type="button" className="btn secondary" data-action="keyboard-close" onClick={() => sheet().close()}>Keep typing</button><button type="button" className="btn primary" data-action="term-give" onClick={() => keyboardAct("give")}>Give back</button></div></>
  ), sheet());
}

/** The pane of run `run` of task `task`, on tmux pane `tmux`; `end` is how an ended run ended. */
export const TerminalPane = memo(function TerminalPane({ run, task, tmux, end }) {
  useStore(painted);
  const { token } = useStore(session);
  const box = useRef(null);
  const term = useRef(null);
  const pre = useRef(null);
  const onScreen = useRef(false);
  const [width, setWidth] = useState(0);
  if (terminal.run !== run) Object.assign(terminal, { run, ...RESET });
  const kept = watch.state();
  // The watch may still hold the run shown before, until it polls this one.
  const { screen, at, stale, gone, more } = kept.run === run ? kept : {};
  // A grant this page holds on a run it no longer shows goes back, so that run's outbox resumes.
  useLayoutEffect(() => {
    keyboard.at(end ? null : run, `${task}/${tmux}`);
  });

  // Polls while a live run's pane is on screen and the page is shown. Leaving, or the page going away,
  // stops the polling and gives a held keyboard back, so the run's outbox does not wait out the 15 minutes.
  useEffect(() => {
    const sync = () => (end || document.hidden || !token || !onScreen.current ? watch.hide() : watch.show(run));
    const observer = new IntersectionObserver(([entry]) => {
      onScreen.current = entry.isIntersecting;
      sync();
    });
    observer.observe(box.current);
    const gone = () => keyboard.leave();
    document.addEventListener("visibilitychange", sync);
    addEventListener("pagehide", gone);
    return () => {
      observer.disconnect();
      document.removeEventListener("visibilitychange", sync);
      removeEventListener("pagehide", gone);
      onScreen.current = false;
      watch.hide();
    };
  }, [run, end, token]);
  useEffect(() => () => keyboard.leave(), []);
  // Escape closes the More menu or the Say line before the document's shortcuts hear it.
  useEffect(() => {
    const element = term.current;
    element.addEventListener("keydown", escape);
    return () => element.removeEventListener("keydown", escape);
  }, []);

  // Fit sizes the text to the pane, between 9 px and the pane's own size, and follows a resize.
  const { fit } = terminal;
  useEffect(() => {
    if (!fit) return undefined;
    const resized = () => setWidth(innerWidth);
    addEventListener("resize", resized);
    return () => removeEventListener("resize", resized);
  }, [fit]);
  const columns = screen?.width;
  useLayoutEffect(() => {
    const element = pre.current;
    Object.assign(element.style, { fontSize: "", lineHeight: "" });
    if (!fit || !columns) return;
    const own = parseFloat(getComputedStyle(element).fontSize);
    Object.assign(element.style, { fontSize: `${Math.min(own, Math.max(9, (element.clientWidth - 24) / (columns * 0.6)))}px`, lineHeight: "1.33" });
  }, [fit, columns, width]);
  useLayoutEffect(() => {
    pre.current.scrollLeft = terminal.left;
  }, [run]);

  const notes = [];
  if (screen?.redacted) notes.push(["warn", "shield", `${plural(screen.redacted, "secret")} hidden on this screen`]);
  if (stale && !end && !gone) notes.push(["warn", "unplug", screen ? `Reconnecting. Screen from ${clock(new Date(at).toISOString())}, ${secondsSince(at)} s ago.` : "Reconnecting. The screen could not be read yet."]);
  if (end) notes.push(["", (STATES[end] || ["muted", "circle-help"])[1], screen ? `Run ${run} ended as ${end}. This is the last screen this page saw.` : `Run ${run} ended as ${end}.`]);
  else if (gone) notes.push(["warn", "unplug", `This server reads no screen of run ${run}. Run priory screen ${task} on the runner to see why.`]);
  else if (!screen && !stale) notes.push(["", "clock", "Loading the screen…"]);
  const keys = keyboard.state();
  const held = end ? null : keys.held;
  const offer = !end && !held && Boolean(screen?.keyboard);
  if (keys.ended && !held) notes.push(["", "clock", keys.ended, offer]);
  const earlier = screen?.earlier || 0;
  const top = end || !screen ? null : more ? <button type="button" className="term-more" data-action="term-earlier" onClick={() => watch.earlier()}><Icon name="chevron-up" size="s15" />Load earlier{earlier ? <>{" "}<span className="dim">{`(${earlier} shown)`}</span></> : null}</button>
    : earlier >= EARLIER_CAP ? <p className="term-edge">{`The last ${EARLIER_CAP.toLocaleString("en")} lines are as far back as this pane reads.`}</p> : null;
  const typing = terminal.typing && !end;
  const copied = (what, text) => (terminal.copied === what ? "Copied" : text);
  const tag = held ? <span className="ro keys">keys</span> : <span className={`ro${typing ? " on" : ""}`}>{typing ? "say" : "read-only"}</span>;
  const menu = terminal.menu ? (
    <div className="term-menu" id="term-more">
      {offer ? <button type="button" data-action="term-take-ask" onClick={() => askKeyboard("take")}><Icon name="keyboard" size="s15" /><span>Take the keyboard<small>Send keys straight into the pane</small></span></button> : null}
      {held ? <button type="button" data-action="term-give-ask" onClick={() => askKeyboard("give")}><Icon name="keyboard" size="s15" /><span>Give the keyboard back</span></button> : null}
      <button type="button" data-action="term-attach" data-task={task} onClick={() => copy(`priory attach ${task} --read-only`, "attach")}><Icon name="eye" size="s15" /><span>{copied("attach", "Copy attach command")}</span></button></div>
  ) : null;
  const sayNow = (event) => {
    event.preventDefault();
    say(task);
  };
  useEffect(() => {
    const dialog = sheet();
    const outside = (event) => { if (event.target === dialog) dialog.close(); };
    dialog.addEventListener("click", outside);
    dialog.addEventListener("close", sheetClosed);
    return () => {
      dialog.removeEventListener("click", outside);
      dialog.removeEventListener("close", sheetClosed);
      if (dialog.open) dialog.close();
    };
  }, []);
  return (
    <div className="termbox" ref={box} data-term={run} data-task={task} data-tmux={tmux} data-end={end}>
      {notes.map(([kind, name, text, again], at) => <p key={at} className={`note ${kind}`} role="status"><Icon name={name} size="s15" /><span>{text}{again ? <>{" "}<button type="button" className="linkish" data-action="term-take-ask" onClick={() => askKeyboard("take")}>Take it again</button></> : null}</span></p>)}
      <div className={`term${terminal.fit ? " fit" : ""}${end ? " ended" : ""}${held ? " held" : ""}`} role="region" aria-label={`Terminal of run ${run}`} ref={term}>
        <div className="term-head"><Icon name="terminal" size="s15" /><span className="mono">{tmux}</span><span className="rid">{`run ${run}`}</span>{end ? <Tile state={end} size="sm" /> : null}{tag}<span className="sp" />
          {end ? null : <button type="button" data-action="term-type" aria-label="Type" aria-pressed={String(typing)} onClick={toggleTyping}><Icon name="message-text" size="s15" /><span>Type</span></button>}
          <button type="button" data-action="term-copy" aria-label="Copy screen" aria-disabled={screen ? undefined : "true"} onClick={() => { if (screen) copy(screenText(screen), "screen"); }}><Icon name="copy" size="s15" /><span>{copied("screen", "Copy screen")}</span></button>
          <button type="button" data-action="term-fit" aria-label="Fit" aria-pressed={String(terminal.fit)} onClick={() => { terminal.fit = !terminal.fit; paint(); }}><Icon name="fit" size="s15" /><span>Fit</span></button>
          <button type="button" data-action="term-menu" aria-label="More" aria-controls="term-more" aria-expanded={String(terminal.menu)} onClick={() => { terminal.menu = !terminal.menu; paint(); }}><Icon name="more" size="s15" /></button></div>
        {menu}{top}
        {/* oxlint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- the screen scrolls sideways, so a keyboard must reach it */}
        <pre id="term-screen" tabIndex="0" aria-label={`Screen${screen ? `, ${screen.width} by ${screen.height}` : ""}`} ref={pre} onScroll={(event) => { terminal.left = event.currentTarget.scrollLeft; }}><Rows screen={screen} end={end} /></pre>
        {typing ? (
          <div className="term-type"><form data-action="term-say" data-task={task} onSubmit={sayNow}><span className="pr" aria-hidden="true">›</span><label className="sr" htmlFor="term-line">Say to the agent</label>
            <input id="term-line" autoComplete="off" placeholder="Say to the agent" value={terminal.line} onChange={(event) => { terminal.line = event.target.value; paint(); }} /><button type="submit" className="send" aria-disabled={terminal.line.trim() && !terminal.saying ? undefined : "true"}>Say</button></form>
            {terminal.said ? <p className={`tn ${terminal.said.said ? "ok" : "no"}`} role="status"><Icon name={terminal.said.said ? "check" : "circle-x"} size="s15" /><span>{terminal.said.note}</span></p> : null}</div>
        ) : null}
        {held ? <KeysBar held={held} keys={keys} /> : null}</div>
      {terminal.sheet ? <KeyboardSheet {...terminal.sheet} /> : null}
    </div>
  );
});
