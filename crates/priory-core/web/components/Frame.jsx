// The frame around a page: the stale strip, the top bar, the navigation and the grid the page sits in.
import { memo, useCallback, useSyncExternalStore } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { leaves } from "../frame.js";
import { PANELS } from "../pages.js";
import { oldestRead, unread } from "../queries.js";
import { failureWords, session } from "../session.js";
import { ago } from "../times.js";
import { useStore } from "../hooks/useData.js";
import { useMinute } from "../hooks/useScreen.js";
import { Icon } from "./Icon.jsx";

export const Connection = memo(function Connection() {
  const off = Boolean(useStore(session).failed);
  return <span className={`conn ${off ? "off" : ""}`}>{off ? "reconnecting" : "live"}</span>;
});

// What a page says when one of its listings could not be read again, while answers still go.
function useUnread(cache) {
  const watch = useCallback((changed) => cache.getQueryCache().subscribe(changed), [cache]);
  const words = () => {
    const found = unread(cache);
    if (!found) return null;
    const why = failureWords(found.error);
    return found.held ? `Could not read the ${found.name} again: ${why}. This is its last read.` : `Could not read the ${found.name}: ${why}.`;
  };
  return useSyncExternalStore(watch, words, words);
}

export const StaleStrip = memo(function StaleStrip({ onRetry }) {
  const { failed } = useStore(session);
  const cache = useQueryClient();
  const page = useUnread(cache);
  useMinute();
  if (!failed && !page) return null;
  const retry = <button type="button" className="ghost" data-retry onClick={onRetry}>Retry</button>;
  if (!failed) return <div className="stale" role="status"><Icon name="unplug" size="s15" /><span>{page}</span>{retry}</div>;
  const read = oldestRead(cache);
  const since = read ? `Last update ${ago(new Date(read).toISOString())}.` : "";
  return <div className="stale" role="status"><Icon name="unplug" size="s15" /><span>{`Reconnecting: ${failed}. ${since}`}</span>{retry}</div>;
});

const TopBar = memo(function TopBar({ factory }) {
  return (
    <header className="wtop">
      <span className="fac">{factory}</span><Connection /><span className="sp" />
      <button type="button" className="addtop" data-action="add-open" aria-label="Add a task" aria-keyshortcuts="N" aria-haspopup="dialog"><Icon name="plus" size="s20" />Add</button>
    </header>
  );
});

const Tab = memo(function Tab({ panel, current, open }) {
  const [label, name] = PANELS[panel];
  return (
    <a href={panel === "desk" ? "/" : `/${panel}`} aria-current={current === panel ? "page" : "false"}>
      <Icon name={name} size="s20" />{label}{panel === "desk" && open ? <span className="badge num" aria-label={`${open} open asks`}>{open}</span> : null}
    </a>
  );
});

/** The navigation: a rail on a wide screen, else a bar of the first four panels and More. */
export const Nav = memo(function Nav({ layout, current, rail, open }) {
  const panels = [...new Set(leaves(layout))];
  const shown = rail ? panels : panels.slice(0, 4);
  const tabs = shown.map((panel) => <Tab key={panel} panel={panel} current={current} open={open} />);
  if (rail) return <nav className="rail" aria-label="Panels">{tabs}<a className="settings" href="/settings/appearance" aria-current={current === "settings" ? "page" : "false"}><Icon name="settings" size="s20" />Settings</a></nav>;
  const more = current === "more" || current === "settings" || (PANELS[current] && !shown.includes(current));
  return <nav className="tabbar" aria-label="Panels">{tabs}<a href="/more" aria-current={more ? "page" : "false"}><Icon name="more" size="s20" />More</a></nav>;
});

export const Frame = memo(function Frame({ shell, size, views, open, onRetry, children }) {
  const { layout } = views.views.default;
  const strip = <StaleStrip key="strip" onRetry={onRetry} />;
  const compact = size === "compact";
  const wide = size === "wide";
  // One tree for every size, so a rotation keeps the page's elements.
  return (
    <>
      {strip}{compact ? null : <TopBar factory={views.factory} />}
      <div className={compact ? "flat" : shell.grid}>{wide ? <Nav rail layout={layout} current={shell.nav} open={open} /> : null}{children}</div>
      {wide || (compact && !shell.nav) ? null : <Nav layout={layout} current={shell.nav} open={open} />}
    </>
  );
});
