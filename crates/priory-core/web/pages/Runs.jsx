// Runs (DESIGN.md, Runs panel): the runs of the last 7 days, newest first; each row opens its task.
import { memo, useEffect } from "react";
import { RUN_FACETS, RUN_VIEWS, duration, runState, runViews, tokenText } from "../runs.js";
import { filtered, olderPart, taskLabel } from "../tasks.js";
import { askFor, asked } from "../legacy/data.js";
import { Icon, StateWord, Tile } from "../components/Icon.jsx";
import { Facets, OlderRow, PageLoading, RowsList, RowsPage, facetPicks, facetQuery, listView } from "../components/Lists.jsx";
import { SavedViews } from "../components/SavedViews.jsx";
import { Time } from "../components/Text.jsx";
import { useRuns, useStore } from "../hooks/useData.js";
import { useMinute } from "../hooks/useScreen.js";
import { usePlace } from "./place.js";

// A table's cell is the mono one, a list's link.
const KeyLink = ({ run, cell }) => <a className={cell ? undefined : "mono"} href={`/tasks/${run.task}`} title={`task ${run.task}`}>{taskLabel(run.key)}</a>;

const RunListRow = memo(function RunListRow({ run, now }) {
  const tokens = tokenText(run.tokens);
  return (
    <div className="row" data-href={`/tasks/${run.task}`}><Tile state={runState(run)} />
      <span className="l1"><KeyLink run={run} /><span>{run.step}</span><span>{runState(run)}</span><span className="age"><Time iso={run.started} /></span></span>
      <span className="l2 r">{[run.agent, run.model].filter(Boolean).join(", ") || "no agent"}</span>
      <span className="l3"><span>{`run ${run.run}`}</span>{run.outcome ? <span>{run.outcome}</span> : null}<span>{duration(run, now)}</span>{tokens ? <span>{`${tokens} tokens`}</span> : null}</span>
      <span className="chev"><Icon name="chevron-right" size="s15" /></span></div>
  );
});

const RunTableRow = memo(function RunTableRow({ run, now }) {
  return (
    <tr data-href={`/tasks/${run.task}`}><td><StateWord state={runState(run)} /></td><td className="r">{run.run}</td><td className="mono"><KeyLink run={run} cell /></td><td>{run.step}</td>
      <td>{`${run.agent || ""} `}<span className="t3">{run.model || ""}</span></td><td>{run.outcome || ""}</td><td className="r">{duration(run, now)}</td><td className="r">{tokenText(run.tokens)}</td><td className="r"><Time iso={run.started} /></td></tr>
  );
});

// The runs listing, with its runs older than 7 days once the person scrolled to them.
function useRunList() {
  const wanted = useStore(asked);
  const { href } = usePlace();
  const current = listView(href, RUN_VIEWS);
  // A live run's duration moves with the clock.
  useMinute();
  useEffect(() => {
    if (current !== "recent" && wanted.all.runs) askFor({ all: { ...wanted.all, runs: false } });
  }, [current, wanted]);
  return { ...useRuns(wanted), href, current };
}

function body({ data, href, current }, wide) {
  const picks = facetPicks(href, RUN_FACETS);
  const all = data.runs ?? [];
  const views = runViews(filtered(all, picks, RUN_FACETS));
  const { counted, unloaded } = olderPart({ view: current, history: "recent", older: data.older, picked: picks.size > 0 });
  const chips = [
    { key: "recent", label: "Recent", count: views.recent.length + counted, on: current === "recent" },
    { key: "live", label: "Live", count: views.live.length, on: current === "live" },
  ];
  const head = <><SavedViews views={chips} base="/runs" facets={facetQuery(picks)} /><Facets view={current} picks={picks} specs={RUN_FACETS} base="/runs" items={all} /></>;
  const list = views[current];
  const empty = current === "live" ? "No run is live. A run shows here while an agent works on a step." : "No run in the last 7 days. A run shows here once a task's step starts an agent.";
  if (!list.length && !unloaded) return { head, rows: <p className="empty">{empty}</p> };
  const now = Date.now();
  const rows = wide
    ? <table className="tbl runs"><thead><tr><th>State</th><th className="r">Run</th><th>Task</th><th>Step</th><th>Agent and model</th><th>Outcome</th><th className="r">Duration</th><th className="r">Tokens</th><th className="r">Started</th></tr></thead><tbody>{list.map((run) => <RunTableRow key={run.run} run={run} now={now} />)}<OlderRow listing="runs" count={unloaded} wide columns={9} /></tbody></table>
    : <>{list.map((run) => <RunListRow key={run.run} run={run} now={now} />)}<OlderRow listing="runs" count={unloaded} /></>;
  return { head, rows };
}

/** Runs beside other panels. */
export const RunsList = memo(function RunsList() {
  const read = useRunList();
  const { size } = usePlace();
  if (!read.data) return <PageLoading error={read.error} />;
  const { head, rows } = body(read, size === "wide");
  return <RowsList label="Runs" href={read.href} views={head}>{rows}</RowsList>;
});

/** Runs alone on a phone. */
export const RunsPage = memo(function RunsPage() {
  const read = useRunList();
  if (!read.data) return <PageLoading error={read.error} />;
  const { head, rows } = body(read, false);
  return <RowsPage label="Runs" href={read.href} views={head}>{rows}</RowsPage>;
});
