/** The pull request a github.com URL shows, or null. @param {string} href */
export function parsePr(href) {
  let url;
  try {
    url = new URL(href);
  } catch {
    return null;
  }
  if (url.origin !== "https://github.com") return null;
  const match = url.pathname.match(/^\/([\w.-]+)\/([\w.-]+)\/pull\/(\d+)(?:\/(files|commits|checks|changes))?\/?/);
  if (!match) return null;
  const [, owner, name, number, tab] = match;
  const repo = `${owner}/${name}`;
  return {
    repo,
    number: Number(number),
    resource: `github:${repo}#${number}`,
    tab: tab === "changes" ? "files" : tab || "conversation",
    url: `https://github.com/${repo}/pull/${number}`,
  };
}

/** The head commit in a pull request page's embedded data, or null. @param {string} html */
export const headSha = (html) => html.match(/"headSha":"([0-9a-f]{40})"/)?.[1] ?? null;
