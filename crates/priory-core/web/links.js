// Autolinks: `GET /api/links` rules applied to text on render. The text itself never changes.
// adapters/herdr/lib/priory.js keeps the same `autolink`; a test there compares the two.

/**
 * Splits `text` into spans, each `{ text }` or `{ text, url }`. At each point the earliest match
 * wins, and the first rule on a tie. A rule whose URL is not `https:`, whose regex this engine
 * refuses, or that needs `{repo}` when `repo` is null, links nothing.
 * @param {string} text @param {{ pattern: string, url: string }[]} links @param {string | null} repo
 * @returns {{ text: string, url?: string }[]}
 */
export function autolink(text, links, repo) {
  const rules = links.flatMap(({ pattern, url }) => {
    if (!url.startsWith("https://") || (url.includes("{repo}") && !repo)) return [];
    try {
      return [{ regex: new RegExp(pattern, "gu"), url }];
    } catch {
      return [];
    }
  });
  const spans = [];
  let at = 0;
  for (;;) {
    let best = null;
    for (const rule of rules) {
      rule.regex.lastIndex = at;
      const match = rule.regex.exec(text);
      if (match && match[0] && (!best || match.index < best.match.index)) best = { rule, match };
    }
    if (!best) break;
    const { rule, match } = best;
    if (match.index > at) spans.push({ text: text.slice(at, match.index) });
    const url = rule.url
      .replaceAll("{repo}", () => String(repo).split("/").map(encodeURIComponent).join("/"))
      .replace(/\$(\d+)/g, (_, group) => encodeURIComponent(match[Number(group)] ?? ""));
    spans.push({ text: match[0], url });
    at = match.index + match[0].length;
  }
  if (at < text.length) spans.push({ text: text.slice(at) });
  return spans;
}

const escapeText = (value) => String(value).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

/** `text` as escaped HTML with its autolinks. */
export function linked(text, links, repo) {
  return autolink(String(text), links, repo)
    .map((span) => span.url
      ? `<a class="autolink" href="${escapeText(span.url)}" rel="noopener noreferrer" target="_blank">${escapeText(span.text)}</a>`
      : escapeText(span.text))
    .join("");
}
