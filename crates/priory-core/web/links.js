// Autolinks: `GET /api/links` rules applied to text on render. The text itself never changes.
// adapters/herdr/lib/priory.js keeps the same `autolink`; a test there compares the two.

/**
 * Splits `text` into spans, each `{ text }` or `{ text, url }`. At each point the earliest match
 * wins, and the first rule on a tie. A bare `https://` URL links to itself and wins a tie, so no rule
 * links a part of it. A rule whose URL is not `https:`, whose regex this engine refuses, or that
 * needs `{repo}` when `repo` is null, links nothing.
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
  // A bare URL ends at a space, `<`, `>`, `"`, a backtick or `]`, and leaves out the punctuation, `…` and unbalanced
  // `)` that prose puts after it. Its link is its own text, when that parses as an `https:` URL.
  const bare = (/** @type {number} */ from) => {
    const regex = /https:\/\/[^\s<>"`\]]+/gu;
    regex.lastIndex = from;
    for (let match = regex.exec(text); match; match = regex.exec(text)) {
      let url = match[0];
      for (let last = ""; last !== url;) {
        last = url;
        url = url.replace(/[.,;:!?'…]+$/u, "");
        if (url.endsWith(")") && url.split(")").length > url.split("(").length) url = url.slice(0, -1);
      }
      try {
        if (new URL(url).protocol === "https:") return { index: match.index, text: url, url };
      } catch {
        // Not a URL: look for the next one.
      }
    }
    return null;
  };
  const spans = [];
  let at = 0;
  for (;;) {
    let best = bare(at);
    for (const rule of rules) {
      rule.regex.lastIndex = at;
      let match = rule.regex.exec(text);
      while (match && !match[0] && match.index < text.length) {
        rule.regex.lastIndex = match.index + (text.codePointAt(match.index) > 0xffff ? 2 : 1);
        match = rule.regex.exec(text);
      }
      if (!match?.[0] || (best && match.index >= best.index)) continue;
      const found = match;
      const url = rule.url
        .replaceAll("{repo}", () => String(repo).split("/").map(encodeURIComponent).join("/"))
        .replace(/\$(\d+)/g, (_, group) => encodeURIComponent(found[Number(group)] ?? ""));
      best = { index: match.index, text: match[0], url };
    }
    if (!best) break;
    if (best.index > at) spans.push({ text: text.slice(at, best.index) });
    spans.push({ text: best.text, url: best.url });
    at = best.index + best.text.length;
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
