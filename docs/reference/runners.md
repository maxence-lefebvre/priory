# Runners

A runner is a machine that runs agents. Each file `runners/<hostname>.toml`, `.yaml` or `.yml`
describes one, named by the machine's Tailscale hostname. Capacity is measured live, so the file
holds only the platform and the share of the machine its agents may take together.

Schema: [`config/runner.json`](../schemas/config/runner.json).

## Keys

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `platform` | table | yes | | The machine's operating system and CPU architecture | `{ os = "linux" }` |
| `platform.os` | `linux`, `macos` or `windows` | yes | | The operating system | `"linux"` |
| `platform.arch` | string | no | | The CPU architecture | `"x86_64"` |
| `agents` | table | no | 90% of each | The share of the machine the runner's agents may take together. See [agents-slice.md](../daemon/agents-slice.md) | `{ cpu = "50%" }` |
| `agents.cpu` | share | no | `"90%"` | The share of the machine's CPUs they may load | `"50%"` |
| `agents.memory` | share | no | `"90%"` | The share of the machine's memory they may take | `"60%"` |

A share is a number followed by `%`, above 0 and at most 100, such as `"60%"`.

## Example

<!-- example: factory/runners/xps13.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/runner.json
platform = { os = "linux", arch = "x86_64" }
agents = { cpu = "50%", memory = "60%" }   # a laptop in use; omit for 90% of each
```

## Several runners

With two or more runner files, each task is owned by one runner at a time. The daemon is the
runner named by its machine's hostname, and refuses to start on a machine with no runner file:

```text
this machine, `<hostname>`, has no runner file in runners/, and the factory has several runners
```

A task that never started moves to another runner 30 minutes after its runner's last alive
message. `priory give KEY RUNNER` moves any other. See [runners.md](../daemon/runners.md).

## Errors from `priory check`

| Message | Cause |
| ------- | ----- |
| `` missing field `platform` `` | No `platform` table |
| `` missing field `os` `` | `platform` without `os` |
| `` unknown variant `<os>`, expected one of `linux`, `macos`, `windows` `` | An operating system not listed above |
| `` unknown field `<key>`, expected one of ... `` | A key the table above does not list |
| `"<text>" is not a share such as 60% or 90%` | A share without `%`, or outside 0 to 100 |
| `` `<hostname>` is also defined in runners/<hostname>.toml `` | `<hostname>.toml` and `<hostname>.yaml` both exist |
