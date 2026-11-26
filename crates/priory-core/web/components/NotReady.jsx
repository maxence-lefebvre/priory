import { useStore } from "../hooks/useData.js";
import { failureWords, session } from "../session.js";

/** What a part says while its listings are not read: why, when it is the first part that cannot draw. */
export function NotReady({ results, speaks = true }) {
  const { failed } = useStore(session);
  if (!speaks) return null;
  const error = results.find((result) => result.status === "error" && result.data === undefined)?.error;
  const why = failed ?? (error ? failureWords(error) : null);
  return why
    ? <p className="empty">{`Could not load this page: ${why}. Check that `}<code>priory serve</code> or the daemon runs, then retry.</p>
    : <p className="empty" role="status">Loading…</p>;
}
