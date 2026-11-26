// Flows (DESIGN.md, Flows panel): each workflow drawn top to bottom. The factory is read once, so the
// flows read once.
import { memo } from "react";
import { plural } from "../desk.js";
import { KINDS, edgeStyle, historyRows, lockBadge, nodeShape, timeline, ungatedPairs } from "../flows.js";
import { Icon } from "../components/Icon.jsx";
import { PageTop, Panel } from "../components/PhonePage.jsx";
import { useFlows } from "../hooks/useFlows.js";
import { failureWords } from "../session.js";
import { listClass, usePlace } from "./place.js";

const KIND_WORD = { read: "read", reversible: "reversible write", irreversible: "irreversible write", ask: "ask", answer: "answer", verdict: "verdict" };
const BY_WORD = { code: "code decides", model: "a model decides", human: "a person decides", asleep: "asleep, waiting" };
const clip = (text, size) => (text.length > size ? `${text.slice(0, size - 1)}...` : text);

const FlowNode = memo(function FlowNode({ node }) {
  if (node.terminal) {
    return <g transform={`translate(${node.x} ${node.y})`} className="fnode end"><title>{node.label}</title><rect width={node.width} height={node.height} rx={node.height / 2} /><text x={node.width / 2} y={node.height / 2 + 4} textAnchor="middle">{node.label}</text></g>;
  }
  const shape = nodeShape(node.kind, node.width, node.height);
  const second = node.gate ? clip(node.gate, 24) : KIND_WORD[node.kind] ?? node.kind;
  const badge = lockBadge(node.width);
  return (
    <g transform={`translate(${node.x} ${node.y})`} className={`fnode k-${node.kind}`}><title>{`${node.label}: ${KIND_WORD[node.kind] ?? node.kind}${node.gate ? `, ${node.gate}` : ""}`}</title>
      <path d={shape.d} /><text x={node.width / 2} y={node.height / 2 - 3} textAnchor="middle" className="fl1">{clip(node.label, 20)}</text>
      <text x={node.width / 2} y={node.height / 2 + 13} textAnchor="middle" className="fl2">{second}</text>
      {node.kind === "irreversible" ? <Icon name="lock" size="flock" box={{ x: badge.x, y: badge.y, width: badge.size, height: badge.size }} /> : null}</g>
  );
});

const FlowEdge = memo(function FlowEdge({ edge }) {
  const style = edgeStyle(edge.by);
  const { label } = edge;
  return (
    <g className={`fedge by-${edge.by}${edge.ungated ? " ungated" : ""}`}><title>{`${edge.outcome}: ${edge.from} to ${edge.to}, ${BY_WORD[edge.by] ?? edge.by}${edge.ungated ? ", reaches an irreversible step with no person's decision" : ""}`}</title>
      {edge.ungated ? <path d={edge.d} fill="none" strokeWidth={style.width + 6} className="halo" /> : null}
      <path d={edge.d} fill="none" strokeWidth={style.width} strokeDasharray={style.dash || undefined} className="fline" />
      <text x={label.x} y={label.y} textAnchor={label.anchor} className="felabel" transform={label.turned ? `rotate(-90 ${label.x} ${label.y})` : undefined}>{clip(edge.outcome, 14)}</text></g>
  );
});

const Legend = memo(function Legend() {
  return (
    <section className="sect flegend" aria-labelledby="legend-title"><h2 className="sh" id="legend-title">Key</h2>
      <ul className="fkey">{KINDS.map((kind) => <li key={kind}><svg viewBox="0 0 150 52" width="45" height="16" aria-hidden="true" className={`fnode k-${kind}`}><path d={nodeShape(kind, 150, 52).d} /></svg>{KIND_WORD[kind]}</li>)}
        {["code", "model", "human", "asleep"].map((by) => <li key={by}><svg viewBox="0 0 48 8" width="48" height="8" aria-hidden="true" className={`fedge by-${by}`}><path d="M0 4H48" strokeWidth={edgeStyle(by).width} strokeDasharray={edgeStyle(by).dash || undefined} className="fline" /></svg>{BY_WORD[by]}</li>)}
        <li><svg viewBox="0 0 48 12" width="48" height="12" aria-hidden="true" className="fedge ungated"><path d="M0 6H48" strokeWidth="8" className="halo" /></svg>no person&apos;s decision before an irreversible step</li></ul></section>
  );
});

const Outcome = ({ label, value: [count, bound] }) => <td data-label={label}>{count}<span className="bound">{bound}</span></td>;

const History = memo(function History({ rows }) {
  if (!rows.length) return null;
  return (
    <div className="history"><p className="note">Tasks through each irreversible step in the last 180 days, judged within 30 days of it.</p>
      <table className="ctbl rtbl"><thead><tr><th scope="col">Step</th><th scope="col" className="r">Caught</th><th scope="col">Judged wrong</th><th scope="col">Landed wrong</th></tr></thead>
        <tbody>{rows.map((row) => <tr key={row.step}><th scope="row" className="mono">{row.step}</th><td className="r" data-label="Caught">{row.caught}</td><Outcome label="Judged wrong" value={row.judged} /><Outcome label="Landed wrong" value={row.landed} /></tr>)}</tbody></table></div>
  );
});

const Flow = memo(function Flow({ workflow, paths, irreversible }) {
  const drawing = timeline(workflow, ungatedPairs(workflow.name, paths));
  const open = paths.filter((entry) => entry.workflow === workflow.name);
  const id = `flow-${workflow.name.replace(/[^\w-]/g, "_")}`;
  return (
    <section className="sect flow" aria-labelledby={id}><h2 className="sh" id={id}>{workflow.name}<span className="meta num">{plural(workflow.steps.length, "step")}</span></h2>
      <svg className="diagram" viewBox={`0 0 ${drawing.width} ${drawing.height}`} role="img" aria-label={drawing.text}>
        <defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerUnits="userSpaceOnUse" markerWidth="10" markerHeight="10" orient="auto-start-reverse"><path d="M0 1L10 5L0 9z" fill="context-stroke" /></marker></defs>
        {drawing.edges.map((edge, at) => <FlowEdge key={at} edge={edge} />)}{drawing.nodes.map((node, at) => <FlowNode key={at} node={node} />)}</svg>
      <History rows={historyRows(irreversible ?? [], workflow.name)} />
      {open.length ? <div className="unattended" role="note"><p className="t-danger"><Icon name="lock" size="s15" />Reaches an irreversible step with no person&apos;s decision:</p><ul>{open.map((entry) => <li key={entry.path.join(">")} className="mono">{entry.path.join(" > ")}</li>)}</ul></div> : null}
      <details className="astext"><summary>Steps as text</summary><ol>{workflow.steps.map((step) => <li key={step.id}><b className="mono">{step.id}</b>{`, ${KIND_WORD[step.kind] ?? step.kind}, decided by ${step.decides}. `}{step.edges.map((edge) => `${edge.outcome} leads to ${edge.to} (${BY_WORD[edge.by] ?? edge.by}). `).join("")}</li>)}</ol></details></section>
  );
});

export const Flows = memo(function Flows() {
  const { size } = usePlace();
  const { data: flows, error, refetch } = useFlows();
  const head = size === "compact" ? <PageTop title="Flows" /> : <div className="lhead"><h1>Flows</h1></div>;
  let body;
  if (!flows && error) body = <><p className="empty" role="alert">{`The flows did not load: ${failureWords(error)}. Run `}<code>priory check --explain</code> meanwhile.</p><p className="empty"><button type="button" className="btn secondary" onClick={() => refetch()}>Retry</button></p></>;
  else if (!flows) body = <p className="empty" role="status">Loading the flows…</p>;
  else body = <Panel label="Flows"><div className="scroll">{flows.workflows.length ? <><Legend />{flows.workflows.map((workflow) => <Flow key={workflow.name} workflow={workflow} paths={flows.paths} irreversible={flows.irreversible} />)}</> : <p className="empty">This factory has no workflows. Add one under <code>workflows/</code>.</p>}</div></Panel>;
  return <div className={listClass(size, "list")}>{head}{body}</div>;
});
