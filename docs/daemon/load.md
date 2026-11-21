# Load test

The load test measures the daemon and its reads on one state file under load. It seeds 500 tasks
(300 asleep with an open or answered ask, the rest done), 5,000 inbox events, 300 asks and 2,000
trace records. Then it times each path below and fails when one passes its budget.

## Run it

The test is ignored by default, and CI runs it only when you ask. Run it on a release build:

```sh
cargo test -p priory --test load --release -- --ignored --nocapture
LOAD_SCALE=10 cargo test -p priory --test load --release -- --ignored --nocapture
```

`LOAD_SCALE` multiplies every count, so `10` seeds 5,000 tasks, 50,000 inbox events, 3,000 asks and
20,000 trace records. The budgets do not scale. They are the limits the daemon keeps at ten times
the load. The test prints one row per measure and lists those over budget.

## Measures and limits

Milliseconds on one laptop, the slowest of 3 to 5 runs for the CLI and HTTP reads. Each CLI row
includes the process start. Numbers vary by about 2x between runs.

| Measure | Budget | 1x | 10x |
| ------- | ------ | -- | --- |
| `priory daemon --once`, nothing pending | 1,000 | 22 | 90 |
| `priory daemon --once`, routing 100 new issues (200 messages) | 3,000 | 400 | 600 |
| `priory desk --json` | 500 | 27 | 120 |
| `priory report attention` | 500 | 9 | 30 |
| `priory costs` | 500 | 17 | 60 |
| `GET /api/asks` | 500 | 21 | 190 |
| `GET /api/tasks` | 500 | 26 | 190 |
| `GET /api/events` wake after a commit | 250 | 95 | 100 |
| one write, including its revision triggers | 50 | 7 | 6 |

Three limits follow:

- `/api/asks` and `/api/tasks` grow in step with the rows they return, since each row carries
  several lookups. At 10,000 open and recent asks, expect about 650 ms; the 7-day window keeps the count down.
- `/api/events` wakes within one poll, 100 ms, of a commit.
- The seed adds no deliveries or actions, so the load test does not measure the per-ask lookups
  that read them.

## What the load test found

Before schema 57, `priory desk --json` took 600 to 980 ms at 10x. It matched each open ask by
`coalesce(name, id)` and listed the newest trace and inbox entries by `julianday(time)`, and no
index served either, so the desk scanned every ask, trace record and inbox entry. Schema 57 adds
`asks_by_ref`, `trace_by_day` and `inbox_by_day`. The desk now takes about 120 ms at 10x.
