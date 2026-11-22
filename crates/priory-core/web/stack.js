// The same `prStack` as adapters/herdr/lib/priory.js; a test there keeps the two the same.

/** The tile a check draws by its conclusion: one with none has not finished (DESIGN.md, Pull request summary). @param {string | null} conclusion */
export function checkState(conclusion) {
  if (!conclusion) return "running";
  if (conclusion === "success") return "passed";
  return ["skipped", "neutral"].includes(conclusion) ? "skipped" : "failed";
}

/** A review or check state in plain words: `changes_requested` reads "changes requested", `success` "passed". @param {string | null} state */
export const stateWords = (state) => (state === "success" ? "passed" : String(state ?? "").replaceAll("_", " "));

/**
 * A task's `stack` for the views; one pull request keeps the single PR look. Only `latest`, the
 * task's `pull_request`, has known checks.
 * @param {any[] | null | undefined} stack
 * @param {any} [latest]
 * @returns {{ label: string | null, entries: { number: number, url: string, branch: string, base: string, state: string, next: boolean, checks: string | null }[] }}
 */
export function prStack(stack, latest = null) {
  if (!stack || stack.length < 2) return { label: null, entries: [] };
  const next = stack.findIndex((pull) => pull.state === "open");
  const checks = latest?.checks ?? [];
  const failing = checks.filter((/** @type {any} */ check) => check.conclusion && !["success", "skipped", "neutral"].includes(check.conclusion)).length;
  const running = checks.filter((/** @type {any} */ check) => !check.conclusion).length;
  const known = !checks.length ? null
    : failing ? `${failing} of ${checks.length} checks failing`
    : running ? `${running} of ${checks.length} checks running`
    : `${checks.length} checks pass`;
  return {
    label: `${stack.length} PRs`,
    entries: stack.map((pull, at) => ({
      number: pull.number,
      url: `https://github.com/${String(pull.pull_request).replace(/^github:/, "").replace(/#\d+$/, "")}/pull/${pull.number}`,
      branch: pull.branch,
      base: pull.base,
      state: pull.state,
      next: at === next,
      checks: pull.number === latest?.number ? known : null,
    })),
  };
}
