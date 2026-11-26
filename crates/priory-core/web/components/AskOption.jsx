// One option of an ask, in DESIGN.md's answer states: idle, armed, and sending on the pressed option
// while the others take no press. The web view's answer sheet and the GitHub extension's panel draw it.

export const Busy = () => <span className="busy" aria-hidden="true" />;

/** `label` replaces the option's words, as an armed option does; `hint` is its key. */
export function AskOption({ option, index, recommended, runs, sending, armed, off, note, label, hint, describedBy, onPress }) {
  const notes = sending ? "Sending…" : note ?? [recommended && "Recommended", runs && "Runs a command"].filter(Boolean).join(". ");
  return (
    <button type="button" className={`opt ${recommended ? "rec-opt" : ""} ${sending ? "chosen" : ""} ${armed ? "armed" : ""}`} data-option={index} data-answer={index} aria-disabled={off ? "true" : "false"} aria-busy={sending ? "true" : undefined} aria-describedby={describedBy} onClick={onPress}>
      {sending ? <Busy /> : null}{label ?? <span>{option}{notes ? <small>{notes}</small> : null}</span>}{hint ? <kbd>{hint}</kbd> : null}
    </button>
  );
}
