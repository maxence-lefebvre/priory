// The root route: the sign-in, the first load, then the frame around the page the address names.
import { memo, useEffect, useMemo, useRef } from "react";
import { Outlet, useRouterState } from "@tanstack/react-router";
import { AddSheet, openAdd } from "../components/AddSheet.jsx";
import { Frame, StaleStrip } from "../components/Frame.jsx";
import { parts, pathOf, shellOf, where } from "../frame.js";
import { useDesk, useKeptTask, useKeptTasks, useLinks, useStore, useViews } from "../hooks/useData.js";
import { useShortcuts } from "../hooks/useShortcuts.js";
import { useSize } from "../hooks/useScreen.js";
import { pageTitle } from "../pages.js";
import { placeAt, replace } from "../router.js";
import { session, signIn } from "../session.js";
import { refresh } from "../data.js";
import { taskLabel } from "../tasks.js";
import { Icon } from "../components/Icon.jsx";
import { Place } from "./place.js";

const submitToken = (event) => {
  event.preventDefault();
  const token = new FormData(event.currentTarget).get("token").trim();
  if (token) signIn(token);
};

export const SignIn = memo(function SignIn({ error }) {
  const field = useRef(null);
  useEffect(() => field.current?.focus(), []);
  return (
    <form className="signin" onSubmit={submitToken}>
      <h1>Priory</h1>
      <p>Paste the token this factory&apos;s daemon was started with. It is the value of <code>PRIORY_TOKEN</code>, or the variable <code>[serve] token_env</code> names. This device keeps it until you sign out.</p>
      <label htmlFor="token">Token</label>
      <input id="token" name="token" type="password" autoComplete="current-password" required ref={field} />
      {error ? <p className="field-error"><Icon name="circle-x" size="s15" />{error}</p> : null}
      <button className="btn primary" type="submit">Sign in</button>
    </form>
  );
});

export const Loading = memo(function Loading({ failed }) {
  if (!failed) return <p className="empty" role="status">Loading the desk…</p>;
  return <><StaleStrip onRetry={refresh} /><p className="empty">{`Could not load the desk: ${failed}. Check that `}<code>priory serve</code> or the daemon runs, then retry.</p></>;
});

// Says a new ask to a screen reader: one this session has not seen open.
function useAnnounce(desk) {
  const seen = useRef(null);
  useEffect(() => {
    if (!desk) return;
    const asks = desk.items.filter((item) => item.kind === "ask");
    const fresh = seen.current && asks.find((ask) => !seen.current.has(ask.id));
    seen.current = new Set([...(seen.current ?? []), ...asks.map((ask) => ask.id)]);
    if (fresh) document.getElementById("announce").textContent = `New ask from ${fresh.key ? taskLabel(fresh.key) : `task ${fresh.task}`}.`;
  }, [desk]);
}

const Signed = memo(function Signed() {
  const location = useRouterState({ select: (state) => state.location });
  const { failed } = useStore(session);
  const size = useSize();
  const views = useViews().data;
  const desk = useDesk().data;
  useLinks();
  // A phone opens no ask on arrival: only a wide screen draws the drawer.
  const place = placeAt(location, desk, size === "wide");
  const at = useMemo(() => where(place.href), [place.href]);
  const task = useKeptTask(at.task);
  const tasks = useKeptTasks();
  useAnnounce(desk);
  useShortcuts(at, place.trail.listUrl);
  useEffect(() => {
    if (place.replace) replace(place.replace);
  }, [place]);
  useEffect(() => {
    document.title = desk ? pageTitle(pathOf(place.href), { tasks: [...(tasks ?? []), task].filter(Boolean) }) : "Priory";
  }, [place.href, desk, tasks, task]);
  const layout = views?.views.default.layout;
  const list = place.trail.list;
  const shell = useMemo(() => layout && shellOf(at, { size, list, layout }), [at, size, list, layout]);
  const section = parts(place.href)[1];
  const value = useMemo(() => shell && { href: place.href, entry: place.entry, trail: place.trail, at, size, shell, section, layout }, [place, at, size, shell, section, layout]);
  if (!views || !desk) return <><Loading failed={failed} /><AddSheet /></>;
  const open = desk.items.filter((item) => item.kind === "ask").length;
  return <><Place.Provider value={value}><Frame shell={shell} size={size} views={views} open={open} onRetry={refresh} onAdd={openAdd}><Outlet /></Frame></Place.Provider><AddSheet /></>;
});

export const Shell = memo(function Shell() {
  const { token, signInError } = useStore(session);
  useEffect(() => {
    if (!token) document.title = "Priory";
  }, [token]);
  return <div id="app">{token ? <Signed /> : <SignIn error={signInError} />}</div>;
});
