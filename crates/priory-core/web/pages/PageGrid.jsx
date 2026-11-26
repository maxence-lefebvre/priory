// The page inside the frame: one slot per panel and one for the drawer. Each slot holds its page's part.
import { memo, useMemo } from "react";
import { Icon } from "../components/Icon.jsx";
import { leaves, partsOf, shownTab } from "../frame.js";
import { PANELS } from "../pages.js";
import { reactPart } from "../registry.js";
import { NotFound } from "./NotFound.jsx";
import { usePlace } from "./place.js";

const Slot = memo(function Slot({ kind, name, id, panels }) {
  const Part = reactPart({ kind, name });
  return Part ? <Part key={id ?? name} id={id} panels={panels} /> : <NotFound />;
});

// The view file's tree, for a layout that is not a plain set of tabs. Its first
// panel is the main slot's, which a smaller screen draws alone.
const Tree = memo(function Tree({ node, list }) {
  if (node.panel) return <Slot kind="list" name={node.panel} />;
  const [kind, children] = Object.entries(node)[0];
  if (kind !== "tabs") return <div className={`tree tree-${kind}`}>{children.map((child, i) => <Tree key={i} node={child} list={list} />)}</div>;
  const open = shownTab(children, list);
  return (
    <div className="tree-tabs">
      <nav className="tabbar" aria-label="Tabs">{children.map((child) => <TreeTab key={leaves(child)[0]} panel={leaves(child)[0]} open={child === open} />)}</nav>
      <Tree node={open} list={list} />
    </div>
  );
});

const TreeTab = memo(function TreeTab({ panel, open }) {
  const [label, name] = PANELS[panel];
  return <a href={panel === "desk" ? "/" : `/${panel}`} aria-current={open ? "page" : "false"}><Icon name={name} size="s20" />{label}</a>;
});

export const PageGrid = memo(function PageGrid() {
  const { at, shell, size, trail, layout } = usePlace();
  const { here, ask, task, batch } = at;
  // An ask keeps the list it was opened from; another page names its own.
  const list = ask ? trail.list : here;
  const { main, drawer } = useMemo(() => partsOf({ here, ask, task, batch }, shell, { size, layout }), [here, ask, task, batch, shell, size, layout]);
  return (
    <>
      {main.kind ? <Slot {...main} /> : <Tree node={main} list={list} />}
      {drawer ? <Slot kind="drawer" name="asks" id={ask} panels={shell.panels} /> : null}
    </>
  );
});
