import { createElement, memo } from "react";
import { ICONS, STATES } from "../icons.js";

// The strokes as elements, read once from this repository's own constants.
const strokes = new Map();
function strokesOf(name) {
  if (!strokes.has(name)) {
    const found = [...(ICONS[name] ?? "").matchAll(/<(\w+) ([^/]*)\/>/g)];
    strokes.set(name, found.map(([, tag, attributes], key) => createElement(tag, { key, ...Object.fromEntries([...attributes.matchAll(/([\w-]+)="([^"]*)"/g)].map(([, attribute, value]) => [attribute, value])) })));
  }
  return strokes.get(name);
}

export const Icon = memo(function Icon({ name, size = "", label }) {
  const named = label ? { role: "img", "aria-label": label } : { "aria-hidden": "true" };
  return <svg className={`ico ${size}`} viewBox="0 0 24 24" {...named}>{strokesOf(name)}</svg>;
});

const stateOf = (state) => STATES[state] || ["muted", "circle-help"];

export const Tile = memo(function Tile({ state, size = "" }) {
  const [kind, name] = stateOf(state);
  return <span className={`tile ${kind} ${size}`} role="img" aria-label={state}>{name ? <Icon name={name} /> : <i className="dot" />}</span>;
});

export const StateWord = memo(function StateWord({ state }) {
  const [kind, name] = stateOf(state);
  return <span className={`sti t-${kind}`}>{name ? <Icon name={name} size="s15" /> : <i className="dot" />}{state}</span>;
});
