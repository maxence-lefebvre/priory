# Decisions

This factory sorts review feedback with TypeSafe's Jev before the feedback wakes an agent. A
comment that Jev sorts as `nothing-to-do`, with a probability of at least 0.9, wakes no one. It
reaches the agent with the next prompt that wakes it for another reason. Copy the
`[decisions.triage]` table and the two files into your own factory.

## What it needs

`python3` on `PATH`, and a TypeSafe API key in `TYPESAFE_API_KEY` for the daemon and for
`priory calibrate`. The comments leave your machine for TypeSafe's API.

## Calibrate it

Jev's answers count only after it sorts your labeled cases as you did:

```sh
TYPESAFE_API_KEY=... priory calibrate triage examples/decisions
```

Read each row where Jev and the label differ. Fix the label, or add cases, and calibrate again.
Editing `triage.eval.toml`, `run` or `threshold` needs a new calibration.

## Gates

None: the factory has no workflow. Until it is calibrated, and whenever a call fails, feedback wakes
the agent as it does with no decision model. See [the design note](../../docs/design/deciders.md).
