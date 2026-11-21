# Prompt evaluations

`priory eval` runs an evaluation file against a real model and counts what the agent did.
[evaluating-a-prompt.md](../evaluating-a-prompt.md) shows how to use it.

## `priory eval`

```sh
priory eval FILE --budget USD [--case NAME] [--runs N] [--model MODEL]
```

| Flag | Default | Meaning |
| ---- | ------- | ------- |
| `--budget USD` | required | The most the evaluation may spend, in dollars. It stops starting runs once it has spent this, and passes what is left to each turn as Claude's `--max-budget-usd`. |
| `--case NAME` | every case | Run only this case. The doc is left alone. |
| `--runs N` | `5` | Runs per case and variant. |
| `--model MODEL` | `haiku` | The model, as Claude's `--model` takes it. |

Each turn is a headless `claude -p` in a scratch folder under the system's temporary folder, with
no built-in tools and no settings or MCP servers of yours. Its one MCP server is
`priory mcp --eval`. A later turn of the same run resumes the first turn's session.

The command prints a Markdown table: one row per case and variant, with the expected ending, the
hits out of the runs, and every ending the agent chose with its count. When every case ran within
the budget and the file names a `doc`, the table replaces the lines between the doc's markers.

A run's ending is its agent's first `result`, `ask` or `wait` call after the prompt under test:

| Ending | The agent called |
| ------ | ---------------- |
| `result <outcome>` | `result`, with that outcome |
| `ask` | `ask` |
| `wait <type>` | `wait`, with that condition type in `on`, or `any` for a list |
| `no exit` | none of the three |
| `<ending> before the prompt` | one of the three in a `before` turn, so the prompt was never typed |

Errors:

- `give --budget USD, the most the evaluation may spend`: the command has no `--budget`.
- `variant NAME needs one of text or file`: a variant has both keys, or neither.
- `DOC has no line <!-- priory eval NAME -->`: the doc lacks the opening marker. `NAME` is the
  evaluation file's name, such as `nudge.eval.toml`.
- `claude gave no session and cost`: a turn's `claude -p` printed no JSON answer. Its output
  follows.

## The evaluation file

A TOML file, named `<prompt>.eval.toml` and kept beside the text it tests.

| Key | Type | Meaning |
| --- | ---- | ------- |
| `tool` | string, optional | A tool of Priory's MCP server whose description each variant replaces. Unset, each variant fills `{prompt}`. |
| `doc` | path, optional | The doc whose table the command rewrites, relative to this file. |
| `[[variants]]` | table list | The texts to compare, in the table's order. |
| `variants.name` | string | The variant's name in the table, such as `old`. |
| `variants.text` | string | The variant's text. |
| `variants.file` | path | Instead of `text`: a file that holds it, relative to this file. Trailing whitespace is dropped. |
| `[[cases]]` | table list | The situations to run. |
| `cases.name` | string | The case's name, for the table and `--case`. |
| `cases.before` | string list, optional | Prompts typed first, one turn each. |
| `cases.prompt` | string | The prompt under test. `{prompt}` stands for the variant's text. |
| `cases.expect` | string | The ending a good run has, such as `result passed`. |

```toml
doc = "../../../docs/daemon/signals.md"

[[variants]]
name = "old"
file = "nudge.txt"

[[variants]]
name = "new"
text = "Your last turn ended in text only. End this turn by calling one tool of the priory MCP server."

[[cases]]
name = "waiting-on-checks"
before = ["You opened pull request 88 in acme/shop. Its checks take 20 minutes. Say what remains."]
prompt = "{prompt}\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: landed, failed."
expect = "wait checks"
```

## `priory mcp --eval`

```sh
priory mcp --eval LOG [--describe TOOL TEXT]
```

The fake MCP server that `priory eval` gives the agent. It serves the same tools as
`priory mcp --session`, appends each `tools/call` params object to `LOG` as one JSON line, and
answers every call with `ok`. `--describe` replaces the description of `TOOL` with `TEXT`.
