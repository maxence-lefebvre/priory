# Herdr from view files

This page explains how R8.6 could work: "A second view host, such as a Herdr layout, renders the
same view files as an adapter." It builds nothing. It holds the model, the first code slice with
its tests, the slices after it, and the open questions with a recommendation for each.

The first slice is built, with each recommendation taken: [herdr.md](../reference/herdr-panels.md#the-layout)
describes it. Its placement differs from the tables below: a map in `layout.js` puts narrow panels in
asides and wide ones in center tabs, and a view's nesting only orders them. It also differs here: `adapters/herdr/layout` prints the layout, and only
`install` writes it.

The later slices are built too, each with its open question's recommendation taken:
[herdr.md](../installing-the-herdr-adapter.md#installing) describes one layout per view, and
[When the views change](../installing-the-herdr-adapter.md#when-the-views-change) the Priory bar's offer to
regenerate. They differ here: `default` keeps `priory.json`, not `priory-default.json`, so its
layout name and `keys.json` bindings stay. The Priory bar regenerates with Herdr's file API and the
installed `priory.json` as the frame, since a copied mod has no `install` to run.

## What existed when this note was written

- A view is a file in `views/`: a tree of `row`, `column` and `tabs` nodes whose leaves are panels
  ([views.md](../reference/views.md)). Priory ships `default`, a `tabs` root of eight panels. The web
  view lays out the view named `default`, and `GET /api/views` serves every view's tree as JSON.
- The Herdr adapter ships mods for Desk, Asks, Queue, Bonjour (the Reports panel), Costs, Flows and
  Task, and one layout, `adapters/herdr/layouts/priory.json`, written by hand
  ([herdr.md](../adapters/herdr.md)). `install` links that file into Herdr's layouts folder.
- That layout is Herdr's own JSON: nested `row`, `column`, `stack` and `block` nodes, each with an
  `id`, where a block holds a mod (`{"mod": "priory-asks"}`) or Herdr's active tab. It frames the
  Priory part with Herdr's chrome: the sidebar, the top bar with the Priory bar and the tab bar,
  and the status line.
- The Priory bar, `priory-tabs`, hard-codes which panels open as center tabs (Desk, Bonjour, Costs,
  Flows) and which sit in the side panel stack (Asks, Queue).
- Mods read and act through the `priory` CLI, on the machine `settings.json` names. They never
  read the factory's files.

So a factory that changes `views/default.toml` changes the web view, and Herdr shows the same thing
as before.

## The model

### A generator, not a layout read at run time

Two ways were weighed:

| Way | For | Against |
| --- | --- | ------- |
| A generator turns a view into Herdr layout JSON, and `install` writes it | Herdr gets a plain layout file, as it expects; the result can be read and tested as JSON | A view change needs the generator run again |
| The mods read `/api/views` and build the layout at run time | A view change shows without a step | Herdr loads a layout from a file. The commands the adapter runs today, `layout.select`, `layout.show` and `layout.hide`, change existing blocks; none adds one. So a mod would have to rewrite the layout file anyway |

The recommendation is the generator. It lives in the adapter, in JavaScript, beside the mods, so the
adapter keeps depending on no Priory crate. It reads the views from a new `priory views --json`,
which prints what `GET /api/views` serves, so it needs no running server and no token.

### What each node maps to

The view tree replaces the hand-written body of the layout, the block `work-body`. The Herdr chrome
around it, the sidebar, the top bar and the status line, stays the adapter's frame, because it is
Herdr's and not a Priory panel.

| View node | Herdr node |
| --------- | ---------- |
| `row` | `row`, children with equal `weight` |
| `column` | `column`, children with equal `weight` |
| `tabs` | `stack`, with the first child `active`. The Priory bar shows one button per child of each stack, which selects it |
| `panel` | `block` with the panel's mod |

Every `tabs` maps the same way, at the root or below it, and whatever its children are. A center
tab of Herdr's holds one mod, so it could not hold a `row` child of a root `tabs`. A stack holds any
node.

The `main` block, Herdr's active tab, always stays: it is where terminals open. When the view has a
`terminal` leaf, `main` takes its place. Otherwise the work body is a `row` of `main` and the view's
tree, as today's layout is a `row` of `main` and the side panel. So the shipped `default` view, a
`tabs` root, becomes one stack beside `main`, with eight buttons in the Priory bar.

Generated ids are the path in the tree, such as `view-0-1`, so they stay stable while the file
does. Navigation uses these ids, not mod names. Today `priory-tabs` finds an open tab by its mod,
which cannot tell apart three `priory-elsewhere` blocks, or one panel placed twice. The generated
layout therefore gives every block its id and the panel's name in its `props`, and a request such
as `priory.asks.open` goes to the first block whose panel is `asks`. The Desk's Answer action, which
today selects `asks` in the side stack, selects that block in its stack instead.

### What each panel maps to

| Panel | Herdr |
| ----- | ----- |
| `desk` | `priory-desk` |
| `asks` | `priory-asks` |
| `queue` | `priory-queue` |
| `reports` | `priory-bonjour` |
| `costs` | `priory-costs` |
| `workflow` | `priory-flows` |
| `terminal` | Herdr's own active tab, the `main` block. Herdr owns terminals, and tmux owns the runs (R5.6) |
| `tasks` | No mod yet. `priory-task` shows one task's detail, not the list |
| `runs` | No mod yet |
| `inbox` | No mod yet |

### What cannot map

- **Panels without a mod**: `tasks`, `runs` and `inbox`. The generator puts a small
  `priory-elsewhere` block in their place, which names the panel and links to it in the web view
  when `settings.json` gives the web view's address. A layout that silently dropped them would hide
  that the view asked for them.
- **The phone layout.** The web view turns every leaf into a tab on a phone. Herdr runs on a desktop,
  so the generator lays the tree out as written.
- **Sizes.** View files carry no sizes, and Herdr wants a weight or pixels. Equal weights, with
  the `min` and `max` the adapter uses today for a side panel, are the default.
- **Two `terminal` panels.** Herdr has one active-tab block, so a second `terminal` leaf
  maps to the same `main` block. The generator refuses a view with two, with the view's file.
- **Panel options.** Views have none today. If a view later gains them, such as a saved desk
  filter, each mod reads its own block's `props`, which Herdr passes already.

## The first slice: generate the layout from `default`

1. `priory views [DIR] --json` prints `{ "factory", "views" }`, the same object as `/api/views`.
2. `adapters/herdr/lib/layout.js` exports `layout(view, frame)`: it takes one view's tree and the
   frame, which is today's `priory.json` with `work-body` emptied, and returns Herdr's layout. It
   follows the two tables above.
3. `adapters/herdr/layout` reads the views JSON on its standard input and writes `priory.json` into
   Herdr's layouts folder. Its input is only that JSON, so it does not care where Priory runs:
   `priory views --json | adapters/herdr/layout` on Priory's machine, or
   `ssh desktop priory views --json | adapters/herdr/layout` from another. `install` takes the same
   input in place of linking the static file. Because the file is generated, `install` refuses to
   overwrite a `priory.json` it did not write, as it refuses for links today.
4. `priory-tabs` reads its buttons from the generated layout's stacks, in place of its two
   hard-coded lists, and finds blocks by id and panel, not by mod. The Desk's Answer action and
   `priory.asks.open` find the Asks block the same way.
5. The `priory-elsewhere` mod, a few lines: the panel's name and a link.

Tests:

- `node --test`: `layout` on the shipped `default` view gives a `row` of `main` and one stack of
  eight blocks, Desk first and active, with `priory-elsewhere` for Tasks, Runs and Inbox, each
  with its own id and panel.
- A `tabs` whose child is a `row` gives a stack whose child is that `row`.
- The Desk's Answer action, on a layout from `default`, selects the Asks block in its stack.
- `layout` on the example in views.md, a `row` of `desk` and a `column` of `queue` over `runs`,
  gives a `row` with a `priory-desk` block and a `column` of `priory-queue` and `priory-elsewhere`.
- The frame's chrome ids, `sidebar`, `priory`, `tabs`, `main` and `status`, are in every output.
- Two `terminal` leaves are refused with the view's name.
- Generated ids are the same on a second run.
- `install --dry-run` against the fake `priory` prints the layout it would write, and refuses a
  `priory.json` that is not its own.
- A Rust CLI test: `priory views --json` on each example factory equals `/api/views`'s body.
- A real Herdr check, by hand: the generated `default` layout opens and every button shows its mod.

## Later slices

1. Mods for `tasks`, `runs` and `inbox`, each from a design board first, as AGENTS.md asks for UI
   work. Each one replaces a `priory-elsewhere` block.
2. Every view, not only `default`: one layout per view, `priory-<view>.json`, so Herdr's layout
   picker offers each.
3. Regenerate when the views change: `priory-tabs` compares the views' JSON with the one it
   generated from at start, and offers to regenerate.

## Open questions

1. **Which view does Herdr use?** Recommendation: `default`, as the web view does, then every view
   as its own layout once slice 2 lands. A Herdr-only view name, such as `herdr`, would split what
   R8.6 asks to keep the same.
2. **Is losing the center tabs acceptable?** Today Desk, Bonjour, Costs and Flows open as center
   tabs, as wide as the window, and Asks and Queue sit in a side stack. Following `default`, all
   eight sit in one stack beside `main`, which the person resizes up to its `max`. Recommendation:
   follow the file, since every panel works at `--pane-narrow`. A person who wants a wider desk
   writes a `row` into `views/default.toml`, and the web view gets the same layout.
3. **Should view nodes gain sizes?** Recommendation: not yet. Add an optional `size` to view nodes
   only once a real view needs one, and then for both hosts.
4. **What runs the generator?** `install` is a shell script, and today the adapter needs Node.js
   only for its tests. Herdr's `exec`, which reaches Priory on another machine, exists only inside a
   mod. Recommendation: keep `layout.js` free of Node and Herdr APIs, with the views JSON as its only
   input. Node runs it for `install` and the tests, reading the JSON from a pipe as above. Slice 3
   runs it inside `priory-tabs`, which fetches the JSON with `exec`. Installing then needs Node.js,
   which the adapter's README must say.
