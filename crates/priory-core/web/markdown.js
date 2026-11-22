// Agent text is untrusted: every character is escaped, and no raw HTML passes through.
import { linked } from "./links.js";

const LISTS = [{ tag: "ul", item: /^\s*[-*+]\s+(.*)$/ }, { tag: "ol", item: /^\s*\d+[.)]\s+(.*)$/ }];
const BOX = /^\[([ xX])\]\s+(.*)$/;
const CHECK = `<svg class="ico s15" viewBox="0 0 24 24" aria-hidden="true"><path d="M20 6 9 17l-5-5"/></svg>`;
const FENCE = /^\s*```/;
// A label holds no `[`, so a line of unmatched brackets scans in linear time.
const INLINE = /`(?<code>[^`]+)`|\*\*(?<bold>[^*]+)\*\*|(?<image>!?)\[(?<label>[^[\]]*)\]\((?<url>https:\/\/(?:[^\s()]|\([^\s()]*\))+)\)/g;
const HEADING = /^\s{0,3}(#{1,6})\s+(.*)$/;
const hostOf = (url) => {
  try {
    return new URL(url).host;
  } catch {
    return null;
  }
};
const span = ({ 0: source, groups: { code, bold, image, label, url } }, inline) => {
  if (code) return `<code>${escaped(code)}</code>`;
  if (bold) return `<strong>${inline(bold)}</strong>`;
  const host = hostOf(url);
  if (!host) return escaped(source);
  const shown = image
    ? `<span class="img">${escaped(label)}</span>`
    : `<a href="${escaped(url)}" rel="noopener noreferrer" target="_blank">${escaped(label)}</a>`;
  return `${shown} <span class="host">${escaped(host)}</span>`;
};
const escaped = (text) => linked(text, [], null);
const startsBlock = (line) => FENCE.test(line) || HEADING.test(line) || LISTS.some(({ item }) => item.test(line));

/** `text` as HTML: the DESIGN.md Markdown subset, with the factory's autolinks. */
export function markdown(text, links, repo) {
  const lines = String(text ?? "").replace(/\r\n?/g, "\n").split("\n");
  const inline = (line) => {
    let html = "";
    let at = 0;
    for (const match of line.matchAll(INLINE)) {
      html += linked(line.slice(at, match.index), links, repo) + span(match, inline);
      at = match.index + match[0].length;
    }
    return html + linked(line.slice(at), links, repo);
  };
  const out = [];
  for (let at = 0; at < lines.length;) {
    const line = lines[at];
    if (!line.trim()) {
      at += 1;
      continue;
    }
    if (FENCE.test(line)) {
      const code = [];
      for (at += 1; at < lines.length && !FENCE.test(lines[at]); at += 1) code.push(lines[at]);
      at += 1;
      out.push(`<pre><code>${escaped(code.join("\n"))}</code></pre>`);
      continue;
    }
    const heading = line.match(HEADING);
    if (heading) {
      const tag = `h${Math.min(heading[1].length, 3) + 2}`;
      out.push(`<${tag}>${inline(heading[2].replace(/\s+#+\s*$/, "").trim())}</${tag}>`);
      at += 1;
      continue;
    }
    const list = LISTS.find(({ item }) => item.test(line));
    if (list) {
      const items = [];
      for (; at < lines.length && list.item.test(lines[at]); at += 1) items.push(lines[at].match(list.item)[1]);
      const boxes = items.map((item) => item.match(BOX));
      const checks = list.tag === "ul" && boxes.every(Boolean);
      const shown = items.map((item, i) => {
        if (!checks) return `<li>${inline(item)}</li>`;
        const [, mark, rest] = boxes[i];
        return mark === " " ? `<li><span class="box" aria-hidden="true"></span><span><span class="visually-hidden">Not done: </span>${inline(rest)}</span></li>` : `<li class="done">${CHECK}<span><span class="visually-hidden">Done: </span>${inline(rest)}</span></li>`;
      });
      out.push(`<${list.tag}${checks ? ` class="checks"` : ""}>${shown.join("")}</${list.tag}>`);
      continue;
    }
    const para = [];
    for (; at < lines.length && lines[at].trim() && !startsBlock(lines[at]); at += 1) para.push(inline(lines[at]));
    out.push(`<p>${para.join("<br>")}</p>`);
  }
  return out.join("");
}
