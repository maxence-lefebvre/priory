# Search

A person or an agent finds tasks by what they are about: "who else touches the done gate?", "did a
task already do this?". Search reads each task's title, request, TL;DR and results.

## TL;DR

A task's TL;DR is a short summary of what it is about, kept beside its request. Until a task has
one, search uses its request's title and opening text instead, cut at 180 words. Four writers
can set it:

1. A request can carry `tldr`, such as `priory request --tldr TEXT`. The summarizer then skips the
   task.
2. The summarizer writes it once, when the task is created, from the request alone (see below).
3. The task's agent can replace it with the `tldr` tool of the priory MCP server, when its work
   drifted from the request. No prompt asks it to.
4. `priory tldr TASK TEXT` sets it by hand.

Every writer goes through one limit. With the in-process `model` on, Priory counts tokens with the
model's tokenizer, and the limit is the input the model was trained on (256 tokens for
all-MiniLM-L6-v2). Otherwise the limit is 180 words. The tool and the CLI refuse a longer TL;DR
with its count and the limit. A summarizer answer and a request's `tldr` are cut, word by word, to
fit. Counting needs only the model's tokenizer, a small file apart from its weights, so it never
waits for the model to load. The daemon's embedding thread downloads both. Until the tokenizer is on
disk, every writer counts words; the daemon looks for it again every 10 seconds.

## The summarizer

`[search] tldr` in `factory.toml` names the summarizer agent, which needs
`harness = "claude-code"`. Its model is the agent's, or `haiku` when the agent sets none. Without
it, no summarizer runs, and new tasks keep the fallback.

The daemon calls it as it calls the stall handler: one headless `claude -p --output-format json`
call in `PRIORY_HOME/summaries`, outside the task's runs, with the variables of a surrounding
Claude session removed, no settings or MCP servers of the user's, and no session kept. Reusing that
path keeps one scrubbed way to call a model once, and its JSON reports the model, tokens and cost.
The call runs in the background, at most two at once, for at most two minutes, so it never blocks
or delays the task. A task can start before its TL;DR arrives.

A call that fails, times out or answers nothing leaves the fallback. A TL;DR another writer set
while the call ran wins. Each call appends a `factory.summarizer.called` record to the trace with
its status, model, tokens and cost, so `priory report --cost` shows it as step `summarizer`. A
daemon restart reads the answer of a call the last daemon left, if it wrote one.

Tasks stored before search came get no summary.

## Keyword search

Keyword search is always on. An SQLite FTS5 index covers each task's title, request, TL;DR and
result summaries and evidence, with the Porter stemmer. Triggers keep it current when a task is
created, its TL;DR changes or a result is stored. A query matches any of its words and ranks by
bm25.

```
priory search QUERY [--state STATE[,STATE...]] [--limit N]
```

prints `task`, `key`, `state`, `step` and the TL;DR, best match first, 10 rows unless `--limit`
says otherwise. `--state` keeps tasks in the states it names. The `search(query, states?, limit?)`
tool returns the same table.

The agent's own `priory mcp` process answers the `tldr` and `search` tools from the state file, as
the CLI does, instead of sending them to the daemon. A slow embedder command or a model loading
then holds up that agent's call alone, never the daemon.

## Embeddings

Embeddings are opt-in:

```toml
[search]
tldr = "summarizer"                                # optional: the summarizer agent
embeddings = true                                  # default false
model = "sentence-transformers/all-MiniLM-L6-v2"   # the default model
# command = "my-embedder"                          # instead of model
```

`command` runs with `sh -c`, reads one text on stdin and prints a JSON array of floats. Each call
may take 30 seconds. `priory check` refuses a `[search]` table that names both `model` and
`command`.

With embeddings on, a daemon thread embeds each task's TL;DR, or its fallback, and stores the
vector with the embedder's name and the text it read. It looks for new or changed texts every
second. Changing `model` or `command` makes every stored vector stale, so the thread embeds every
task again once. A search compares only vectors of the current embedder and of the task's current
text, so a task whose TL;DR changed matches by keyword alone until the thread reaches it.

A search embeds the query with the same embedder and compares it with each stored vector one by
one in Rust, with cosine similarity. A task matches by vector at 0.3 or more. The keyword and the
vector rankings then merge by reciprocal rank fusion: a task scores `1 / (60 + rank)` in each
ranking it is in, so a match from both rankings adds both scores. Each ranking hands the fusion
its best 200 tasks, or `--limit` of them when that is larger.

A missing model, a failing command or a download failure never blocks a task. The daemon logs it
once, records why, and tries again a minute later. Until then `priory search` matches by keyword,
and says on stderr that embeddings are unavailable and why. The `search` tool says so in its
first line.

## The in-process model

`model` runs in process with [candle](https://github.com/huggingface/candle) and the Hugging Face
`tokenizers` crate, both pure Rust, on the CPU. Priory downloads the model's `config.json`,
`tokenizer.json`, `sentence_bert_config.json` and `model.safetensors` with `curl` at first use, into
`PRIORY_HOME/models`, and loads them from there afterwards. Only the daemon's embedding thread
downloads, with `curl` bounded at 30 minutes per file, each file into
a temporary file of its own that it renames when complete. Nothing is fetched at build time, so a
Nix build without network works. It pools the token vectors by their mean and normalizes the result,
as sentence-transformers does. Runtimes such as `ort` (ONNX Runtime) download a native library at
build time, which a Nix sandbox refuses.

The runtime sits behind the `model` cargo feature, off by default: `cargo build --release
--features model`. On a 12-core machine it adds about 25 seconds to a clean release build (2 minutes
2 seconds to 2 minutes 27 seconds of wall time, and 3.2 times the CPU time), and it grows the binary
from 9.9 MB to 18.3 MB. all-MiniLM-L6-v2 takes 88 MB on disk and 30 to 45 ms per TL;DR on that
CPU. A priory built without the feature reports embeddings unavailable for `model`, and works with
`command`.

`priory search` and the `search` tool load their own copy of the model for each query, about 0.15
seconds with its files on disk. Before the daemon has downloaded them, they match by keyword and say
that the model is not downloaded yet.
