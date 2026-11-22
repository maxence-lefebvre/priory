# How to install the Herdr adapter

This guide shows how to install Priory's panels into Herdr, keep their layouts in step with the
factory's views, and use Herdr on another machine. It assumes a clone of this repository and
Node.js on the machine that runs `install`. [How the Herdr adapter
works](adapters/herdr.md) explains what the install sets up.

## Installing

Installing needs Node.js. Pipe the views into `install` on the machine Herdr runs on:

```sh
priory views --json | adapters/herdr/install
ssh desktop priory views --json | adapters/herdr/install   # Priory on another machine
```

`install` links each folder in `adapters/herdr/mods/` into Herdr's mods folder, and writes one
layout per view into its layouts folder: `default` as `layouts/priory.json`, and each other view as
`layouts/priory-NAME.json`, so Herdr's layout picker offers each. `default` keeps `priory.json`, so
its layout name and its `keys.json` bindings stay `priory`. Herdr's folder is
`$HERDR_APP_HOME`, or `~/.config/herdr-app`. The links point into your checkout, so a `git pull`
updates the mods, and Herdr reloads them when their files change. The layouts are files: after you
change a view, run `install` again, or click the Priory bar's notice (see
[When the views change](#when-the-views-change)). Each root id ends in a hash of the views `install`
read, such as `priory-generated-00bc386a7e08`. `install` says `layout is up to date with the view files` when
the layout it replaces has the same hash, and `layout was older than the view files, and is
rewritten` when not. `priory check` warns, and still passes, when the layout in Herdr's folder on
its machine has another hash than the factory's views. A layout on another machine is not checked.

- `install --dry-run` prints each link it would make, then `write` and the layout for each view,
  and changes nothing.
- `install` removes a `layouts/priory-NAME.json` it wrote for a view that is gone: a file whose
  root id is its own. It prints `remove` and the path, and `--dry-run` only prints. It leaves every
  other layout alone.
- `install --copy` copies each mod folder in place of linking it, and writes `.priory-installed`
  into the copy, holding the adapter's Git commit (`unknown` outside a checkout). Use it when
  Herdr's folder reaches another machine through Syncthing, which carries no links, such as a Herdr
  on Windows. A later `install --copy` replaces a folder that holds that marker, files that left
  the source included, and a link into this adapter. It stops on any other folder. A plain
  `install` stops on a copy, so the two never mix. A copy you made by hand, without the marker,
  is not the script's: after you check one is a copy of this adapter, mark it with
  `git -C adapters/herdr rev-parse HEAD > ~/.config/herdr-app/mods/priory-desk/.priory-installed`,
  one folder at a time, then run `install --copy`.
- A link that already points into this adapter stays as it is. The layout is written again on
  every run.
- `install` removes what an earlier install made of a mod this adapter no longer ships: a folder
  that holds `.priory-installed`, or a link into this adapter. It prints `remove` and the path, and
  `--dry-run` only prints. It leaves every other mod in Herdr's folder alone.
- Any other file or link at a mod's target stops the script before it changes anything. So does a
  layout file for a view that `install` did not write: a file whose JSON root id is not
  `priory-generated`, alone or with 12 hex digits after a hyphen, or, for `priory.json`, a link elsewhere than to this adapter. Move that file away and run it
  again.
- Input that is not `priory views --json`, a node or panel Priory does not ship, or a view with
  two `terminal` panels stops the script before it changes anything.
- `install` writes the layout beside its target and renames it over the target, so a failed write
  keeps the old layout.

## When the views change

When it starts, the Priory bar runs `priory views --json` and compares its hash with the one in the
shown layout's root id. When they differ, it posts one notice, "The view files changed". Clicking it
runs `priory-tabs.regenerate`, which takes the same steps as `install` for the layouts, in the
layouts folder of the Herdr that shows the bar: it writes every view's layout and removes its own
layout of a view that is gone. It never regenerates without that click.

- It uses the installed `layouts/priory.json` as the frame, so a frame change in a newer adapter
  still needs `install`.
- It refuses, before writing anything, a layout file it did not write, and says which in an error
  notice. Move that file away, then regenerate again.
- It does not update the mods. Run `install` for those.

## Installing for a Herdr on another machine

The mods run on the machine that runs Herdr, and `priory` runs where the `priory` object of
`settings.json` says: its `machine` is the Herdr machine that runs the program, such as `"desktop"`.
Copy the mods with `install --copy` on the machine whose folder Syncthing shares, and set `machine`
in `settings.json`, which Herdr reads from the same folder, to the machine that runs Priory. Without
`machine`, a mod runs `priory` on the machine Herdr's window runs on, which is the Windows machine
and has no Priory.
