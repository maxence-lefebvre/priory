// The same `prStack` as adapters/herdr/lib/priory.js; a test there keeps the two the same.

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
  const failing = checks.filter((/** @type {any} */ check) => !["success", "skipped", "neutral"].includes(check.conclusion)).length;
  const known = !checks.length ? null : failing ? `${failing} of ${checks.length} checks failing` : `${checks.length} checks pass`;
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
