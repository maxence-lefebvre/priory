# Decision models

This page explains how Priory could ask a small, fast model typed questions, such as a choice
between three options or a yes or no, and act on the answers' probabilities. It builds nothing. It
holds the interface, the calibration, the fallback, the caps, the trace, the decision points in
the order they would ship, the first code slice with its tests, and the open questions with a
recommendation for each.

Principle 2 of the [PRD](../prd.md#principles) does not move: code decides what code can decide,
and models judge. A decision model judges. Priory's code reads its answer against a threshold a
person set, and decides.

## The models this note was written against

- **Jev**, TypeSafe's "System One" model ([docs](https://docs.typesafe.ai)). `POST /v1/systemone`
  takes a `model`, a `state` and named `questions`. A question is a `choice` among named options,
  a `score` on ordered levels, or a `noul`, a yes or no. Each answer carries `probabilities` for
  each option and a `confidence` derived from them, and the response names the versioned model
  that answered, such as `jev-1.14.0`. It is priced per input token, at $0.042 per million in
  2026-11, with 32k tokens of context per question. An alias such as `jev-latest` moves when a
  release ships, so its answers can change with no change on the caller's side.
- **Clef**, Cloudflare's decision models on Workers AI
  ([announcement](https://blog.cloudflare.com/clef-decision-models/)). They return a choice with
  per-option probabilities, through the Workers AI binding or its REST API. Workers AI also serves
  Jev.
- **Perplexity's Sonar** models answer to a JSON schema
  ([structured outputs](https://docs.perplexity.ai/docs/grounded-llm/output-control/structured-outputs)),
  with no probabilities. A script can still fit one behind the interface below, with a probability
  of 1 for its answer, and calibration then measures it like any other.

The three differ in endpoint, request and pricing. So Priory talks to none of them: it runs a
command, as it runs a sweep script or a `command` scorer.

## What existed when this note was written

- An agent sorts review feedback. A wake that carries a review, a pull request comment, a review
  thread comment, an issue comment or a Linear comment adds a rule to the prompt: sort each item by
  its URL as `trivial`, `nothing-to-do` or `persons-call`. The result's `feedback` field holds the
  classes, and the task keeps them as `triage` ([tools.md](../daemon/tools.md)). Every such item
  wakes a sleeping task: it fires a `pull_request` or `review` wait, or the checks gate before
  `done` ([waits.md](../daemon/waits.md#feedback-on-a-pull-request)).
- One kind of feedback already wakes nothing: a review whose `state` is `approved` or `dismissed`
  asks the agent for nothing, so it fires no wait and no gate, and reopens no done task
  (`settles` in `crates/priory-core/src/tools.rs`).
- A judge scorer's verdict counts only once `priory score --calibrate` has stored its agreement
  with a person's labeled cases, at or above its `agreement`, as a `factory.scorer.calibrated`
  record that holds the hashes of its prompt and cases ([score.md](../daemon/score.md#calibration)).
  Editing either makes the judge not calibrated again.
- The factory names each secret by its environment variable, such as `token_env` and `secret_env`,
  and never holds the value ([factory.md](../reference/factory.md)).
- "Decider" already has a meaning: each step's decider is `code`, `model` or `human`
  ([PRD](../prd.md#workflows), [trace schema](../trace-schema.md)). This note calls the new thing a
  decision model, so the two never meet in one sentence. See open question 1.

## The interface

### What a factory names

A table in `factory.toml` per decision point. The table's name is the point:

```toml
[decisions.triage]
run = "python3 decisions/jev.py"           # with sh -c, in the factory's folder
key_env = "TYPESAFE_API_KEY"               # the variable that holds the key, never the key
evaluation = "decisions/triage.eval.toml"  # a person's labeled cases
agreement = 0.9                            # the least share of cases it must sort as the person did
threshold = 0.9                            # the least probability Priory acts on
timeout = "5s"                             # at most 30s
daily_calls = 200                          # calls the daemon makes in a local day
```

`priory check` refuses an unknown point, a missing `run`, `key_env` or `evaluation`, an
`agreement` or `threshold` outside 0.5 to 1, a `timeout` over 30 seconds, `daily_calls = 0`, and a
`key_env` that is not a variable name. It refuses an evaluation file that breaks its point's rule for cases.

### What Priory sends and reads

Priory runs `run` with the question as JSON on stdin, in Jev's shape:

```json
{
  "state": {"kind": "github.pull_request_review_comment", "author": "ana", "body": "Thanks, looks good.", "path": "src/lib.rs"},
  "questions": {
    "triage": {
      "type": "choice",
      "instructions": "How should the agent that wrote this pull request treat this feedback?",
      "criteria": {
        "trivial": "One obvious change inside the pull request's scope",
        "persons-call": "A behaviour, design or scope change, a disagreement, or anything unclear",
        "nothing-to-do": "Thanks, an approval, or a remark that asks for no change"
      }
    }
  }
}
```

and reads the answer from stdout, in Jev's shape too:

```json
{
  "model": "jev-1.14.0",
  "answers": {"triage": {"type": "choice", "choice": "nothing-to-do", "probabilities": {"trivial": 0.02, "persons-call": 0.01, "nothing-to-do": 0.97}}},
  "usage": {"input_tokens": 212}
}
```

So the script for Jev is a short pass-through: it adds the model's name and the key, and posts.
A script for another provider maps both ways. An answer counts only when it names a `model`,
answers every question, picks one of the question's options, and gives that option a probability
between 0 and 1. Its probability is the one Priory compares to `threshold`. Anything else is a
failure.

### The key

The command runs with a cleared environment: `PATH`, `HOME`, and the variable `key_env` names,
read from the daemon's environment at call time. So the script sees no other secret of the
factory, and a key set or rotated after the daemon started is the one used. An unset or empty
variable is a failure, and Priory never reads or logs the value.

## Calibration

A decision model's answers count only once it has agreed with a person at its `agreement`, as a
judge's do, with the same parts:

- **The labels** are a `*.eval.toml` file of cases a person labeled, beside the factory, in the
  shape the point needs. For triage, each case is one feedback item and its class:

  ```toml
  [[cases]]
  name = "thanks"
  kind = "github.pull_request.comment"
  body = "Thanks, looks good to me."
  label = "nothing-to-do"
  why = "Asks for nothing."
  ```

  A point needs at least 10 cases and at least 3 of each class, as a judge needs 10 cases with 3
  of each verdict.
- **The run** is `priory calibrate POINT [DIR] [--json]`. It asks the model each case, in order,
  and prints one row per case, then the agreement, as `priory score --calibrate` does.
- **The record** is a `factory.decision.calibrated` trace record with the point, the agreement,
  the model that answered, the threshold, the SHA-256 of the evaluation file, and a SHA-256 over
  `run`, the files it names in the factory's folder and Priory's question. Editing any of them,
  or the threshold, makes the point not calibrated again.

Two rules add to the judge's, because a decision acts on one class and not on a verdict:

1. **No costly miss.** A case the person labeled other than the class Priory acts on, that the
   model put in that class at or above `threshold`, fails the calibration whatever the agreement.
   For triage, a request for a change that the model calls `nothing-to-do` with 0.95 would have
   been dropped from the agent's view.
2. **The same model.** Every case must be answered by one model, and an answer counts later only
   when its `model` is the calibrated one. An alias that moves to a new release makes the point
   not calibrated until someone calibrates again. The script should send a versioned model.

Calibration calls do not count toward `daily_calls`: a person starts them. Ten cases of a few
hundred tokens each cost a fraction of a cent on Jev.

## The fallback

Each decision point keeps today's behaviour, unchanged, when any of these holds:

- the factory sets no table for it;
- it has no calibration that matches, or one under its `agreement`, or one with a costly miss;
- the call fails: the command cannot start, exits non-zero, runs past `timeout`, prints an answer
  that does not count, or answers with another model;
- the key's variable is unset or empty;
- the day's calls reached `daily_calls`;
- the answer's probability is under `threshold`.

A fallback never blocks: Priory goes on as if no model existed. Each one leaves its reason in the
trace.

## Caps

- **Latency.** `timeout` bounds each call, 5 seconds by default and 30 at most. The first slice
  calls the model while it routes an event, so a slow call delays the events behind it by as much.
- **Cost.** `daily_calls` bounds the calls per day, counted from the trace, since a call's cost
  depends on its provider and Priory cannot price it. A state over 16 KB is not sent, which bounds
  each call's input tokens.
- **Data.** The state leaves the machine for the provider. Setting `key_env` is the factory's
  consent. The person's own comments are never sent: they already go to the person.

## What the trace records

Each call appends one `factory.decision.answered` record: the point, the item it was about (the
feedback's URL), the model, the choice and its probability, whether Priory acted on it, the reason
when it did not, the input tokens the answer reported, and the milliseconds the call took. A
fallback that made no call (not calibrated, over the day's calls, an item too large) appends the
same record with no model and its reason, so a person can see how often each fallback fires. A
factory with no table appends none. The state itself is not recorded: the inbox holds the event it
came from.

## The decision points, in order

### (a) Review-comment triage

Today an agent sorts each feedback item. Most `nothing-to-do` items are thanks or remarks, and
each one still wakes a sleeping task: a run, a worktree and its tokens, to sort one comment and
sleep again.

With a calibrated model, Priory asks it to sort each item the agent would sort, before it routes
the event. Only one answer changes what Priory does:

- **`nothing-to-do` at or above `threshold`**: the item settles. It fires no wait and no checks
  gate, and wakes no subscriber. It stays unread, so the next prompt that wakes the task for
  another reason carries it, already sorted: the rule does not name it, and the next result
  records its class with no word from the agent, unless the agent gives it a class of its own.
  Feedback on a done task is not asked about: it reopens the task as before, since a settled item
  would reach no task.
- **`trivial` or `persons-call`**: today's behaviour. The agent sorts it.

A running task reads the item at once, as today: it is already awake, so there is nothing to save.

`nothing-to-do` is the one class whose action Priory can take without the agent: doing nothing.
`trivial` needs the agent to change code. `persons-call` could become an ask Priory raises itself,
in a later slice, once the trace shows how often the model is right about it.

### (b) Intake scripts' noise and band

A trigger's [`pre`](../reference/triggers.md#pre) script can drop an event as noise or set its
band ([pre.md](../intake/pre.md)). A decision model could answer the same two questions, a `noul`
for noise and a `choice` of band, for a trigger that names a point. The fallback is the trigger
and its `pre` script as written.

### (c) The confidence for approve-by-confidence

[Approve by confidence](approve-by-confidence.md) answers a reversible ask with the agent's
recommendation when the agent states a high enough confidence. That confidence is the word of the
party that wants to go on. A decision model could give a second one: the probability that a
person would pick the recommendation, from the question, the options and the trade-off. The
daemon would then auto-answer only when both are at their levels.

It never widens what is eligible. The same function that decides what a lead may answer decides
what a decision model may weigh in on, so an ask whose answer can let an irreversible step run
never reaches one. A decision model never answers an irreversible step's ask, a human step that
`approves`, or an ask that runs a command.

### (d) A cheaper judge scorer

A judge asks Claude whether a change does what was asked, at a few cents a call. A `noul` on the
request and the diff costs a thousandth of that. A scorer of `kind = "decision"` would calibrate
on the judge's own labeled cases, with the judge's `agreement`, and answer `pass` at or above its
threshold, `fail` under one minus it, and `error` between, so an unsure answer never counts. A
diff over the model's context is `error`, as a diff over 100,000 bytes is for a judge.

## The first slice: triage

1. `[decisions.triage]` in `factory.toml`, its `priory check` rules, and the triage cases' loader.
2. The call: run the command with the cleared environment and the timeout, parse and check the
   answer, and append `factory.decision.answered`.
3. `priory calibrate triage [DIR] [--json]`, which appends `factory.decision.calibrated`.
4. In the daemon, before it routes a feedback item the agent would sort: ask the model when the
   point is calibrated and under its day's calls, and mark the item settled when the answer is
   `nothing-to-do` at the threshold. The mark is Priory's, never the intake's: Priory removes one
   an event arrives with.
5. `examples/decisions/`, a starter with `jev.py`, a short script for Jev with no dependency beyond
   Python's standard library, and ten labeled cases. A fake decision model for the tests.
6. A new [decisions.md](../daemon/decisions.md), [factory.md](../reference/factory.md),
   [cli.md](../reference/cli.md), the [trace schema](../trace-schema.md),
   [tools.md](../daemon/tools.md), [waits.md](../daemon/waits.md) and the config schemas.

Tests, through the CLI with the fake model only. None calls a real API:

- A thanks comment on a waiting task's pull request, sorted `nothing-to-do` at 0.97 by a
  calibrated model, fires no wait. The task's next result records it as `nothing-to-do` with no
  `feedback` field.
- The same comment sorted `nothing-to-do` at 0.6 fires the wait with the rule, as today.
- A comment sorted `trivial` fires the wait with the rule, as today.
- With no `[decisions.triage]`, with no calibration, after the evaluation file changed, when the
  command fails, runs past its timeout or prints a malformed answer, when the key's variable is
  unset, past `daily_calls`, and when the answer names another model: the wait fires as today, and
  the trace holds one `factory.decision.answered` with the reason.
- The command sees the key's variable and no other variable of the daemon's.
- An event that arrives already marked settled by the intake is routed as if it were not.
- `priory calibrate triage` stores the agreement, the model and the hashes, and fails with no
  record when a call fails. A labeled `trivial` case answered `nothing-to-do` at the threshold
  fails the calibration at any agreement.
- `priory check` refuses each bad key and an evaluation file with too few cases of a class.
- A `proptest` feeds the answer parser random and mutated JSON and asserts it returns or refuses
  without panicking.

## Later slices

1. Triage's `persons-call` at the threshold raises the ask itself, naming the item, with no run.
2. The noise and band of a trigger's `pre` script.
3. The second confidence for approve-by-confidence, after its first slice.
4. The `decision` scorer kind.
5. The task page and the desk say which items a decision model sorted.

## Open questions

1. **What is it called?** "Decider" is a step's `code`, `model` or `human`, in the PRD and in each
   trace record. Calling the new thing a decider would make "the step's decider" ambiguous.
   Recommendation: "decision model" in prose, `[decisions.<point>]` in the factory, and
   `factory.decision.*` in the trace. A step that acts on a decision model's answer keeps `code`
   as its decider: code compares the answer to a threshold and decides.
2. **A command, or an HTTP client in Priory?** A client would save a Python process per call, and
   tie Priory to one provider's request and key header. Recommendation: a command. It is the shape
   of sweep scripts and `command` scorers, it needs no dependency, and a provider change is a
   script change.
3. **Should the call leave the routing path?** The first slice blocks routing for up to `timeout`
   per feedback item. Feedback arrives a few times an hour, and Jev answers in well under a
   second. Recommendation: keep it inline in the first slice, and move it to a thread when the
   trace's milliseconds show a p95 over one second.
4. **Both `agreement` and `threshold`?** `agreement` says the model sorts as the person does
   overall. `threshold` says how sure it must be before Priory acts on one item. The costly-miss
   rule ties them: it checks the agreement where acting goes wrong. Recommendation: both, with 0.9
   for each by default, and lower neither until the trace shows how often triage falls back under
   the threshold.
5. **Where do labels come from?** A person writes the first ten cases by hand. The task's `triage`
   holds the agent's classes, which are not a person's. Recommendation: labels from a person only.
   A later verb could turn a feedback item a person sorted into a case.
6. **Should a decision model see untrusted text?** The state is a comment anyone with access to
   the pull request wrote. A decision model generates no text and calls no tool, so a comment can
   at most bend its answer. The costly-miss rule and the threshold bound that, and the item still
   reaches the agent with the next wake. Recommendation: send it, and never send a decision
   model's answer to an agent as an instruction.
