// Costs (DESIGN.md, Costs panel): what the factory spent this week against last week, and what it
// cost the person, from /api/costs and /api/attention.
import { Fragment, memo, useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { SPLITS, attentionView, costTiles, dayBars, money, scoreTable, splitTable } from "../costs.js";
import { searchOf } from "../frame.js";
import { costs as costsRead } from "../reads/costs.js";
import { taskLabel } from "../tasks.js";
import { date } from "../times.js";
import { Icon } from "../components/Icon.jsx";
import { PageTop, Panel } from "../components/PhonePage.jsx";
import { SavedViews } from "../components/SavedViews.jsx";
import { Time } from "../components/Text.jsx";
import { useStore } from "../hooks/useData.js";
import { useAttention, useCosts } from "../hooks/useCosts.js";
import { useMinute } from "../hooks/useScreen.js";
import { failureWords, onSignOut, session } from "../session.js";
import { listClass, usePlace } from "./place.js";

// The split the person picked last, kept from one visit to the next until a sign-out.
let picked = "workflow";
onSignOut(() => { picked = "workflow"; });

const TRENDS = { up: "trending-up", down: "trending-down" };

const Delta = ({ delta }) => (delta ? <span className="delta">{TRENDS[delta.direction] ? <Icon name={TRENDS[delta.direction]} size="s13" /> : null}{delta.text}</span> : <span className="delta">No change to show</span>);

const Stat = memo(function Stat({ tile }) {
  return <div className="stat"><span className="lbl">{tile.label}</span><span className="val num">{tile.value}</span>{"delta" in tile ? <Delta delta={tile.delta} /> : <span className="delta">{tile.note}</span>}</div>;
});

const Pick = memo(function Pick({ by, onPick }) {
  const offline = Boolean(useStore(session).failed);
  return (
    <label className="by"><span>Split by</span><select aria-disabled={offline} disabled={offline} value={by} onChange={(event) => onPick(event.target.value)}>{SPLITS.map((each) => <option key={each} value={each}>{each}</option>)}</select></label>
  );
});

const Scores = memo(function Scores({ costs, pick = null }) {
  const table = scoreTable(costs);
  if (!table.rows.length) return null;
  return (
    <><h3 className="sh" id="costs-scores">{`Scores by ${costs.by}`}{pick}</h3><p className="cap">The pass rate leaves out errors and not applicable.</p>
      <div className="tscroll"><table className="ctbl stbl" aria-labelledby="costs-scores"><thead><tr><th scope="col">{table.dimension}</th><th scope="col">Scorer</th>{["Pass rate", "pass", "fail", "error", "not applicable"].map((label) => <th key={label} scope="col" className="r">{label}</th>)}</tr></thead>
        <tbody>{table.rows.map((row) => <tr key={`${row.name} ${row.scorer}`}><th scope="row" className="mono">{row.name}</th><td className="mono">{row.scorer}</td><td className="r c rate">{row.rate}</td><td className="r c">{row.pass}</td><td className="r c">{row.fail}</td><td className="r c">{row.error}</td><td className="r c">{row.notApplicable}</td><td className="line">{row.line}</td></tr>)}</tbody></table></div></>
  );
});

// Bars in percent of the box, so the chart takes any width without stretching its text.
const DayChart = memo(function DayChart({ costs }) {
  const bars = dayBars(costs);
  const top = Math.max(0, ...costs.days.map((day) => day.cost_usd ?? 0));
  const slot = 100 / bars.length;
  const height = 96;
  const split = bars.findIndex((bar) => bar.current);
  return (
    <figure className="days"><svg width="100%" height={height + 56} role="img" aria-label={`Cost per day, last week then this week. ${bars.map((bar) => bar.text).join(". ")}.`}>
      <text className="axis" x="0" y="10">{top ? money(top) : ""}</text>
      <line className="base" x1="0" x2="100%" y1={16 + height} y2={16 + height} />
      {bars.map((bar, i) => {
        const h = Math.max(bar.fraction * height, 1);
        const x = i * slot;
        return (
          <g key={bar.date} className={bar.current ? "cur" : "prev"}><title>{bar.text}</title>
            <rect x={`${x + slot * 0.18}%`} y={16 + height - h} width={`${slot * 0.64}%`} height={h} rx="2" />
            <text className="d" x={`${x + slot / 2}%`} y={height + 32} textAnchor="middle"><tspan className="wd">{`${bar.weekday} `}</tspan>{bar.date}</text>
            <text className="m" x={`${x + slot / 2}%`} y={height + 48} textAnchor="middle">{bar.merges}</text></g>
        );
      })}
      {split > 0 ? <><line className="week" x1={`${split * slot}%`} x2={`${split * slot}%`} y1="0" y2={height + 52} /><text className="wk" x={`${split * slot}%`} dx="4" y="10">this week</text></> : null}</svg>
      <figcaption>{`Bars show each day's cost${split > 0 ? ", and this week starts at the line" : ""}. The figure under each day counts its merges.`}</figcaption></figure>
  );
});

const Spend = memo(function Spend({ costs, by, onPick }) {
  const table = splitTable(costs);
  const pick = <Pick by={by} onPick={onPick} />;
  if (!costs.days.length && !costs.split.length) {
    // Scores can land in a window with no cost, so they keep their table and its Split by.
    const scored = scoreTable(costs).rows.length > 0;
    return <><p className="empty">No cost in the last two weeks. Costs appear once a model step records its tokens. Price each model in <code>prices.toml</code> in the factory folder.</p>{scored ? <section className="sect" aria-labelledby="costs-scores"><Scores costs={costs} pick={pick} /></section> : null}</>;
  }
  // The legacy markup wrote each run of text as one node: a split one kerns differently.
  const { unpriced: models, unknown } = table;
  const tail = `${models.length ? ` ${models.at(-1).tokens} tokens. Add each model to ` : ""}`;
  const unpriced = models.length || unknown
    ? <div className="note"><p><b>Not in the cost.</b>{models.length ? <>{models.map((entry, i) => <Fragment key={entry.model}>{i ? ` ${models[i - 1].tokens} tokens, ` : " Tokens with no price: "}<span className="mono">{entry.model}</span></Fragment>)}{tail}<code>prices.toml</code>{` to price them. ${unknown ? `${unknown}, so their cost is unknown.` : ""}`}</> : `  ${unknown ? `${unknown}, so their cost is unknown.` : ""}`}</p></div>
    : null;
  return (
    <><section className="sect" aria-labelledby="costs-week"><h2 className="sh" id="costs-week">This week<span className="meta">against last week</span></h2>
      <div className="stats">{costTiles(costs).map((tile) => <Stat key={tile.label} tile={tile} />)}</div>{unpriced}</section>
    <section className="sect" aria-labelledby="costs-days"><h2 className="sh" id="costs-days">Each day</h2><DayChart costs={costs} /></section>
    <section className="sect" aria-labelledby="costs-split"><h2 className="sh" id="costs-split">{`This week by ${costs.by}`}{pick}</h2>
      <div className="tscroll"><table className="ctbl"><thead><tr><th scope="col">{table.dimension}</th><th scope="col" className="r">Cost</th><th scope="col" className="r">Tokens</th><th scope="col">Notes</th></tr></thead>
        <tbody>{table.rows.map((row) => <tr key={row.name}><th scope="row" className="mono">{row.name}</th><td className="r">{row.cost}</td><td className="r">{row.tokens}</td><td className="n">{row.note}</td></tr>)}</tbody></table></div><Scores costs={costs} /></section></>
  );
});

const PARTS = [["working", "Working"], ["person", "Waiting on you"], ["world", "Waiting on the world"], ["queued", "Queued"]];

const TaskBar = memo(function TaskBar({ task }) {
  const shown = task.parts.filter((part) => part.fraction > 0);
  const starts = shown.map((_, i) => shown.slice(0, i).reduce((sum, part) => sum + part.fraction, 0));
  return (
    <svg className="tbar" width="100%" height="12" role="img" aria-label={task.text}>
      {shown.map((part, i) => <rect key={part.part} className={`p-${part.part}`} x={`${starts[i] * 100}%`} y="0.5" width={`${part.fraction * 100}%`} height="11"><title>{`${part.label}: ${part.value}`}</title></rect>)}
      {/* A 2 px line of the page ground between parts keeps them apart in every encoding. */}
      {starts.slice(1).map((edge) => <line key={edge} className="gap" x1={`${edge * 100}%`} x2={`${edge * 100}%`} y1="0" y2="12" />)}</svg>
  );
});

const Attention = memo(function Attention({ report }) {
  if (!report.acts && !report.tasks.length && !report.waits.acted && !report.waits.open) {
    return <p className="empty">No attention counted this week. Your answers, dismissals and snoozes on asks count here, and each task&apos;s time once it runs.</p>;
  }
  const view = attentionView(report);
  const { headline } = view;
  return (
    <><section className="sect" aria-labelledby="attention-head"><h2 className="sh" id="attention-head">Your attention<span className="meta">this week</span></h2>
      <div className="headline"><span className="val num">{headline.value}</span><span className="w">{headline.label}<span>{headline.note}</span></span></div>
      <div className="stats">{view.waits.map((tile) => <Stat key={tile.label} tile={tile} />)}</div></section>
    <section className="sect" aria-labelledby="attention-tasks"><h2 className="sh" id="attention-tasks">Where each task&apos;s time went</h2>
      <ul className="legend">{PARTS.map(([part, label]) => <li key={part}><svg width="18" height="10" aria-hidden="true"><rect className={`p-${part}`} x="0.5" y="0.5" width="17" height="9" rx="2" /></svg>{label}</li>)}</ul>
      <ol className="tsplit">{view.tasks.map((task) => <li key={task.id}><p><a className="mono" href={`/tasks/${task.id}`} title={`task ${task.id}`}>{taskLabel(task.key)}</a><span>{task.workflow}</span><span className="sp" /><span className="num">{task.wall}</span></p><TaskBar task={task} /></li>)}</ol></section>
    <svg width="0" height="0" className="defs" aria-hidden="true"><defs><pattern id="stripe-wait" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><rect className="stripe" width="3" height="6" /></pattern></defs></svg></>
  );
});

// True once `on` has held for 300 ms. Only a first read waits for it, so it never turns back.
function useSlow(on) {
  const [late, setLate] = useState(false);
  useEffect(() => {
    if (!on) return undefined;
    const timer = setTimeout(() => setLate(true), 300);
    return () => clearTimeout(timer);
  }, [on]);
  return on && late;
}

export const Costs = memo(function Costs() {
  const { size, href } = usePlace();
  const [by, setBy] = useState(picked);
  const costs = useCosts(by);
  const attention = useAttention();
  const { failed } = useStore(session);
  const now = useMinute() * 60000;
  const cache = useQueryClient();
  const pickBy = (value) => setBy((picked = value));
  const wide = size === "wide";
  const shown = new URLSearchParams(searchOf(href)).get("view") === "attention" ? "attention" : "costs";
  const error = costs.error ?? attention.error;
  // A split still read draws the last one, as old as that one is.
  const costsAt = costs.data && cache.getQueryState(costsRead(costs.data.by).queryKey)?.dataUpdatedAt;
  const spend = costs.data && attention.data ? { costs: costs.data, attention: attention.data, updated: Math.min(costsAt, attention.dataUpdatedAt) } : null;
  const slow = useSlow(!spend && (costs.isFetching || attention.isFetching));
  const retry = () => {
    costs.refetch();
    attention.refetch();
  };
  const head = size === "compact" ? <PageTop title="Costs" /> : <div className="lhead"><h1>Costs</h1></div>;
  const read = spend && new Date(spend.updated).toISOString();
  let body = null;
  if (spend) {
    const costsPart = <Spend costs={spend.costs} by={by} onPick={pickBy} />;
    const attentionPart = <Attention report={spend.attention} />;
    body = wide ? <div className="spend"><div>{costsPart}</div><div>{attentionPart}</div></div> : shown === "costs" ? costsPart : attentionPart;
  } else if (slow) body = <div className="sk" aria-hidden="true"><span /><span /><span /></div>;
  return (
    <div className={listClass(size, "list")}>{head}
      <Panel label="Costs" data-costs="">
        {wide ? null : <SavedViews views={[{ key: "costs", label: "Costs", count: null, on: shown === "costs" }, { key: "attention", label: "Attention", count: null, on: shown === "attention" }]} base="/costs" />}
        <div className="scroll">
          {error ? <p className="note warn" role="alert"><Icon name="circle-x" size="s15" /><span>{`Could not load costs: ${failureWords(error)}. Check that the daemon runs on its runner, then retry.`}</span><button type="button" className="ghost" onClick={retry}>Retry</button></p> : null}
          {spend && (failed || error || now - spend.updated > 60000) ? <p className="note asof"><span>Costs as read <Time iso={read} text={date(read)} />.</span></p> : null}
          {body}
        </div>
      </Panel>
    </div>
  );
});
