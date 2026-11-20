# Views

A view is a layout of the web view: a tree of rows, columns and tabs whose leaves are panels. Each
file `views/<name>.toml`, `.yaml` or `.yml` holds one view, named by its file. Priory ships the view
`default`; a factory file of the same name replaces it. The web view lays out the view named
`default`.

Schema: [`config/view.json`](../schemas/config/view.json).

## Keys

| Key | Type | Required | Meaning | Example |
| --- | ---- | -------- | ------- | ------- |
| `layout` | node | yes | The tree's root | `{ tabs = [{ panel = "desk" }] }` |

A node is a table with exactly one of these keys:

| Key | Value | Meaning |
| --- | ----- | ------- |
| `row` | array of nodes | The children side by side |
| `column` | array of nodes | The children one above the other |
| `tabs` | array of nodes | One child at a time, picked from a tab bar or the rail |
| `panel` | panel name | A leaf: one panel |

The panels are `desk`, `asks`, `inbox`, `queue`, `tasks`, `runs`, `terminal`, `workflow` and
`reports`.

## The default view

```toml
[layout]
tabs = [
  { panel = "desk" },
  { panel = "asks" },
  { panel = "tasks" },
  { panel = "runs" },
  { panel = "inbox" },
  { panel = "reports" },
]
```

On a phone, every leaf becomes a tab in the tree's order: the bottom bar shows the first four and
More holds the rest. On a wide screen, a `tabs` root becomes the rail beside the list and the
drawer; any other root is laid out as written. See [DESIGN.md](../../DESIGN.md#layout).

## Example

<!-- example: factory/views/default.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/view.json
# The desk beside a column of the queue over the runs, replacing the default view.
[layout]
row = [
  { panel = "desk" },
  { column = [{ panel = "queue" }, { panel = "runs" }] },
]
```

## Errors

`priory check` reports each problem with its file and line:

| Message | Cause |
| ------- | ----- |
| `` unknown variant `kanban`, expected one of `desk`, ... `` | A panel name Priory does not ship |
| `` unknown variant `grid`, expected one of `row`, `column`, `tabs`, `panel` `` | A node key other than the four |
| `a row, column or tabs needs at least one child` | An empty `row`, `column` or `tabs` |
| `` `default` is also defined in views/default.toml `` | `default.toml` and `default.yaml` both exist |

`priory serve` and the daemon read the views when they start. Restart them after a change.
