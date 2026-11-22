# Decision models

A decision model is a command that answers typed questions with a probability for each option,
such as TypeSafe's Jev. The daemon asks one before it routes a person's feedback on a task, and
settles the feedback when the model is sure it asks the agent for nothing. A factory names it in
[`[decisions.triage]`](../reference/factory.md#decisionstriage).
[The design note](../design/deciders.md) explains the choices behind it and the decision points
that come next.

## Triage

Before the daemon routes an event, it removes any `decided` field the event arrived with: only
the daemon marks an item. Then it asks the triage model when all of these hold:

- the event is a feedback item the agent would sort (see [tools.md](tools.md)): a review that is
  not an approval or a dismissal, a pull request comment, a review thread comment, an issue
  comment or a Linear comment, not written by the factory's own login;
- its author is not an outsider (see [routing.md](routing.md));
- a task that is not done owns or subscribes to the pull request, branch, issue or Linear issue it
  is on. Feedback on a done task's pull request reopens it as before;
- the factory names `[decisions.triage]`.

The model gets the item's event type, body, file path and review state, and one `choice`
question among `trivial`, `persons-call` and `nothing-to-do`. When it answers `nothing-to-do` with
a probability at or above `threshold`, the daemon marks the event with `decided`, and the item
settles. It fires no wait and no checks gate, and wakes no task that subscribes to it. It stays
unread. The next prompt that wakes the task for another reason carries it. The sorting rule does
not name it, and the task's next result records it as `nothing-to-do`, unless the agent's
`feedback` gives the item a class of its own, which wins. A running task reads it at once, as
before.

Any other answer leaves the item as it would be with no model: the agent sorts it.

## What Priory sends and reads

The daemon runs `run` with `sh -c` in the factory's folder, writes the question on stdin and reads
the answer from stdout, both as JSON in the shape of Jev's System One API:

```json
{"state": {"kind": "github.pull_request.comment", "body": "Thanks, looks good."},
 "questions": {"triage": {"type": "choice", "instructions": "How should the agent that wrote this pull request treat this feedback?",
   "criteria": {"trivial": "...", "nothing-to-do": "...", "persons-call": "..."}}}}
```

```json
{"model": "jev-1.14.0",
 "answers": {"triage": {"type": "choice", "choice": "nothing-to-do", "probabilities": {"nothing-to-do": 0.97}}},
 "usage": {"input_tokens": 212}}
```

An answer counts when it names a `model`, picks one of the options, and gives that option a
probability from 0 to 1. The command runs with a cleared environment: `PATH`, `HOME`, and the
variable `key_env` names, read when the call starts. An item whose state is over 16 KB is not sent.
[`examples/decisions/jev.py`](../../examples/decisions/jev.py) is such a command for Jev.

## Calibration

The model's answers count only once it sorted a person's labeled cases as the person did:

```sh
TYPESAFE_API_KEY=... priory calibrate triage [DIR] [--json]
```

It asks the model each case of the `evaluation` file, in order, and prints one row per case, then
the agreement:

```text
case	label	choice	probability	agreed	error
thanks	nothing-to-do	nothing-to-do	0.97	true
rename	trivial	trivial	0.95	true
agreement: 10/10 labeled cases, 100%, needs 90%: calibrated
```

It appends a `factory.decision.calibrated` record ([the trace schema](../trace-schema.md#factorydecisioncalibrated))
with the agreement, the model, the threshold, the SHA-256 of the evaluation file, and a SHA-256
over `run`, each file `run` names in the factory's folder, such as `jev.py`, and Priory's question.
A script outside the folder, or a module it imports, is not hashed: name the script by its path
in the folder.
The point counts as calibrated while all of these hold:

- the latest record matches `run`, the evaluation file and `threshold` as they are now;
- its agreement is at least `agreement`;
- it has no miss: no case labeled `trivial` or `persons-call` that the model sorted
  `nothing-to-do` at the threshold, since that item would have been kept from the agent.

A calibration whose call fails, or whose cases two models answered, stores nothing and fails.
A daemon answer counts only when its `model` is the calibrated one, so a model alias that moves to
a new release leaves the point not calibrated until you calibrate again.

## Fallback

The item reaches the agent as it would with no model, with the reason in a
`factory.decision.answered` record ([the trace schema](../trace-schema.md#factorydecisionanswered)),
when:

| `why` | Cause |
| ----- | ----- |
| `not calibrated: ...` | No matching calibration, an agreement under `agreement`, or a miss. No call is made |
| `the day's <n> calls are spent` | The daemon made `daily_calls` calls in this local day, failed ones included. No call is made |
| `the item is over 16384 bytes` | No call is made |
| `<key_env> is unset` | The key's variable is unset or empty |
| `it exited <code>: <first line of stderr>` | The command failed. The line is cut to 200 characters, and the key's value in it reads `<KEY_ENV>` |
| `it ran past <n>s` | The command ran past `timeout`. Priory kills its process group |
| `the answer ...` | The answer is not JSON, names no model, picks no option or gives it no probability |
| `answered by <model>, calibrated on <model>` | Another model answered |
| `under the threshold <n>` | The probability is under `threshold` |
| `Priory acts on nothing-to-do only, not <class>` | The model sorted it `trivial` or `persons-call` |

The call runs while the daemon routes events, so a slow call delays the events behind it, by
`timeout` at most.
