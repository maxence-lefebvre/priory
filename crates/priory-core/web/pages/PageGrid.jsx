// The page inside the frame: one slot per panel and one for the drawer. A slot holds a React part, or a
// legacy one in an element of its own, as its page's entry says.
import { memo, useLayoutEffect, useMemo, useRef } from "react";
import * as legacy from "../app.js";
import { Icon } from "../components/Icon.jsx";
import { leaves, partsIn, partsOf, shownTab } from "../frame.js";
import { useListings, useStore } from "../hooks/useData.js";
import { useMinute } from "../hooks/useScreen.js";
import { asked, land, landing } from "../legacy/data.js";
import { PANELS } from "../pages.js";
import { entryOf } from "../registry.js";
import { shown } from "../shown.js";
import { NotFound } from "./NotFound.jsx";
import { usePlace } from "./place.js";

const FRAME = ["views", "links", "desk"];
const REACT = { list: "List", page: "Page", detail: "Detail", drawer: "Drawer" };

export const reactPart = ({ kind, name }) => entryOf(name)?.[REACT[kind]];
const known = ({ kind, name }) => Boolean(reactPart({ kind, name }) ?? entryOf(name)?.[kind]);

/** The listings a page's legacy parts draw from, beside the frame's own. */
export function needsOf(parts, section) {
  const wanted = parts.filter((part) => !reactPart(part)).flatMap(({ kind, name, id }) => {
    const { needs, detailNeeds } = entryOf(name) ?? {};
    if (kind === "detail" || kind === "drawer") return (id && detailNeeds) || [];
    return (typeof needs === "function" ? needs(section) : needs) ?? [];
  });
  return [...new Set(wanted)].sort();
}

// React renders the element once and never its children. CSS gives it no box of its own.
const LegacySlot = memo(function LegacySlot({ slot, kind, name, id, panels }) {
  const host = useRef(null);
  useLayoutEffect(() => legacy.show(host.current, { slot, kind, name, id, panels }), [slot, kind, name, id, panels]);
  return <div className="legacy" ref={host} />;
});

const Slot = memo(function Slot({ slot, kind, name, id, panels }) {
  const Part = reactPart({ kind, name });
  if (!known({ kind, name })) return <NotFound />;
  return Part ? <Part key={id ?? name} id={id} panels={panels} /> : <LegacySlot slot={slot} kind={kind} name={name} id={id} panels={panels} />;
});

// Lands the listings the legacy slots read, and draws them. It comes after the slots: theirs mount first.
const LegacyOwner = memo(function LegacyOwner({ parts, names, pending }) {
  const { href, entry, trail, size } = usePlace();
  // The frame's listings too: the page lands, and reads what it alone reads, once they all did.
  const { reading, read } = useListings([...FRAME, ...names], useStore(asked));
  const minute = useMinute();
  useLayoutEffect(() => legacy.mount(), []);
  useLayoutEffect(() => legacy.open({ href, entry, trail, size, names }), [href, entry, trail, size, names, parts, pending]);
  // A hidden tab draws nothing: a legacy draw may read.
  const visible = useStore(shown);
  useLayoutEffect(() => {
    if (!reading && visible) land();
  }, [reading, read, minute, visible]);
  return null;
});

// The view file's tree, for a layout that is not a plain set of tabs. `at` names each slot.
const Tree = memo(function Tree({ node, list, at = "list" }) {
  if (node.panel) return <Slot slot={at} kind="list" name={node.panel} />;
  const [kind, children] = Object.entries(node)[0];
  if (kind !== "tabs") return <div className={`tree tree-${kind}`}>{children.map((child, i) => <Tree key={i} node={child} list={list} at={`${at}.${i}`} />)}</div>;
  const open = shownTab(children, list);
  return (
    <div className="tree-tabs">
      <nav className="tabbar" aria-label="Tabs">{children.map((child) => <TreeTab key={leaves(child)[0]} panel={leaves(child)[0]} open={child === open} />)}</nav>
      <Tree node={open} list={list} at={`${at}.tab`} />
    </div>
  );
});

const TreeTab = memo(function TreeTab({ panel, open }) {
  const [label, name] = PANELS[panel];
  return <a href={panel === "desk" ? "/" : `/${panel}`} aria-current={open ? "page" : "false"}><Icon name={name} size="s20" />{label}</a>;
});

export const PageGrid = memo(function PageGrid() {
  const { at, shell, size, trail, section, layout } = usePlace();
  const { here, ask, task, batch } = at;
  // An ask keeps the list it was opened from; another page names its own.
  const list = ask ? trail.list : here;
  const { main, drawer, parts, names } = useMemo(() => {
    const found = partsOf({ here, ask, task, batch }, shell, { size, layout });
    const all = [...partsIn(found.main, list), ...(found.drawer ? [{ kind: "drawer", name: "asks", id: ask }] : [])];
    return { ...found, parts: all, names: needsOf(all, section) };
  }, [here, ask, task, batch, shell, size, layout, list, section]);
  const legacyParts = parts.filter((part) => known(part) && !reactPart(part));
  // A tree of legacy panels alone says once that it loads.
  const pending = useStore(landing);
  const loading = pending && !main.kind && legacyParts.length === parts.length;
  return (
    <>
      {loading ? <LegacySlot slot="list" kind="loading" /> : main.kind ? <Slot slot="list" {...main} /> : <Tree node={main} list={list} />}
      {drawer && !loading ? <Slot slot="drawer" kind="drawer" name="asks" id={ask} panels={shell.panels} /> : null}
      {legacyParts.length ? <LegacyOwner parts={parts} names={names} pending={pending} /> : null}
    </>
  );
});
