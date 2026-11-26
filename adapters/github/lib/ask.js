// One ask's answer, as the web view does it: a 4 s Undo, the revision guard, an armed command option.

export const UNDO_MS = 4000;
export const ARM_MS = 4000;

const BUSY = new Set(["waiting", "posting"]);

/** Whether a person may dismiss an ask: a step's or a hold's ask needs an answer. @param {any} ask */
export const dismissable = (ask) => ask?.dismissable !== false;

/** Whether answering `option` runs the ask's command. */
export const runsCommand = (ask, option) => typeof ask.command === "object" && Boolean(ask.command?.runs?.includes(option));

/** The option index a key press picks: a digit 1 to 9 alone, else -1, so a browser shortcut or a held key picks nothing. */
export const optionKey = (event) =>
  event.ctrlKey || event.metaKey || event.altKey || event.repeat || !/^[1-9]$/.test(event.key) ? -1 : Number(event.key) - 1;

/** "2 min", "3 h" or a date, as the views say how long ago. */
export function age(iso, now) {
  const minutes = Math.max(0, Math.round((now - Date.parse(iso)) / 60000));
  if (minutes < 60) return `${minutes} min`;
  if (minutes < 24 * 60) return `${Math.round(minutes / 60)} h`;
  return new Date(iso).toLocaleString("en-US", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hour12: false });
}

/**
 * The draft after the person chooses an option or sends `text`. Nothing changes while one waits
 * or posts, nor after a lost write unless the person sends again (`again`).
 */
export function choose(draft, ask, { option, text, again = false }, now) {
  if (draft && (BUSY.has(draft.phase) || (draft.phase === "unknown" && !again))) return draft;
  const answer = option === undefined ? { text } : { option };
  const phase = option !== undefined && runsCommand(ask, option) ? "armed" : "waiting";
  // A resend of the same choice keeps its call, so the daemon stores it once.
  const same = draft?.id === ask.id && draft.revision === ask.revision && draft.option === answer.option && draft.text === answer.text;
  const call = same && draft.call ? draft.call : crypto.randomUUID();
  return { id: ask.id, revision: ask.revision, ...answer, phase, at: now, call };
}

/** Undo drops a waiting answer; nothing else can be undone. */
export const undo = (draft) => (draft?.phase === "waiting" ? null : draft);

/** The call to send once the Undo time passed, with the draft now posting, else null. */
export function due(draft, now) {
  if (draft?.phase !== "waiting" || now - draft.at < UNDO_MS) return null;
  return { draft: { ...draft, phase: "posting" }, path: answerPath(draft.id), body: answerBody(draft) };
}

/**
 * The draft the panel keeps for its next mount: an answer still waiting on Undo comes back unsent,
 * for the person to send or discard, since a page change ends its timer. Else null.
 */
export function leftUnsent(draft) {
  if (draft?.restored) return draft;
  if (draft?.phase !== "waiting") return null;
  return { ...draft, phase: "unsent", restored: true, notice: `Not sent: "${draft.option ?? draft.text}". The page changed before its 4 s Undo passed.` };
}

export const answerPath =(id) => `/api/asks/${encodeURIComponent(id)}/answer`;

export const answerBody = (draft, run = false) => ({
  ...(draft.option === undefined ? { text: draft.text } : { option: draft.option }),
  revision: draft.revision,
  ...(draft.call ? { call: draft.call } : {}),
  ...(run ? { run: true } : {}),
});

/** What a refused or lost write says, in the panel's words, and the phase it leaves. */
export function refusal(result, now) {
  const { status, body } = result;
  if (result.unknown) return ["unknown", "This may have reached Priory. Check the task, then send again if it is missing."];
  if (status === 401) return ["signin", "The daemon refused this token. Paste the current one."];
  if (status === 403) return ["unsent", "Priory refused this extension's origin. Add it to [serve] extension_origins."];
  if (status === 503) return ["unsent", "The daemon is not running: nothing was sent."];
  const fresh = body?.ask;
  if (status === 409 && fresh?.state === "open") return ["refused", `The question changed. Read revision ${fresh.revision} and answer again.`];
  if (status === 409 && fresh?.state === "answered") return ["taken", `Already answered ${age(fresh.answered_at, now)} ago: ${fresh.answer}.`];
  if (status === 409 && fresh?.state === "withdrawn") return ["withdrawn", `Withdrawn: ${fresh.withdrawn_why}.`];
  return ["unsent", `${body?.error || `The server answered ${status}`}.`];
}

/** The draft after the daemon's reply to its answer. */
export function reply(draft, result, now) {
  if (result.status === 200) return { ...draft, phase: "sent", notice: null };
  const [phase, notice] = refusal(result, now);
  return { ...draft, phase, notice };
}

/**
 * The draft once a fresh read shows its ask: an armed command disarms at a new revision or after
 * 4 s, a restored draft drops at a new revision, since its choice was made on the old one, and a
 * lost write counts as sent once the ask shows it answered.
 */
export function observe(draft, ask, now) {
  if (!draft || draft.id !== ask.id) return draft;
  if (draft.restored && ask.revision !== draft.revision) return null;
  if (draft.phase === "armed") return ask.revision === draft.revision && ask.state === "open" && now - draft.at < ARM_MS ? draft : null;
  if (draft.phase === "unknown" && ask.state !== "open") return { ...draft, phase: "sent", notice: null };
  return draft;
}
