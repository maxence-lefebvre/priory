import { test } from "node:test";
import assert from "node:assert/strict";
import { markdown } from "./markdown.js";

test("renders paragraphs and a bullet list", () => {
  const text = "Checks are green.\n\n- Emails retry 3 times.\n- The tests pass.";

  assert.equal(markdown(text, [], null), "<p>Checks are green.</p><ul><li>Emails retry 3 times.</li><li>The tests pass.</li></ul>");
});

test("escapes raw HTML in text, items and code", () => {
  const text = `<script>alert(1)</script>\n\n- <img src=x onerror="a()">\n\n\`\`\`\n<b>&</b>\n\`\`\``;

  assert.equal(
    markdown(text, [], null),
    "<p>&#60;script&#62;alert(1)&#60;/script&#62;</p><ul><li>&#60;img src=x onerror=&#34;a()&#34;&#62;</li></ul><pre><code>&#60;b&#62;&#38;&#60;/b&#62;</code></pre>",
  );
});

test("keeps a fenced block's lines and links nothing inside it", () => {
  const text = "Run:\n```sh\ncargo test  # ENG-7\n  --workspace\n```";
  const links = [{ pattern: String.raw`\b[A-Z]{2,5}-\d+\b`, url: "https://linear.app/i/$0" }];

  assert.equal(markdown(text, links, null), "<p>Run:</p><pre><code>cargo test  # ENG-7\n  --workspace</code></pre>");
});

test("renders inline code without autolinks and links the text around it", () => {
  const links = [{ pattern: String.raw`\b[A-Z]{2,5}-\d+\b`, url: "https://linear.app/i/$0" }];

  assert.equal(
    markdown("ENG-7 adds `ENG-8 <x>` here", links, null),
    `<p><a class="autolink" href="https://linear.app/i/ENG-7" rel="noopener noreferrer" target="_blank">ENG-7</a> adds <code>ENG-8 &#60;x&#62;</code> here</p>`,
  );
});

test("links only https targets, with the host after the text", () => {
  const text = `See [the "run"](https://ci.example.com/r?a=1&b=2), [bad](javascript:alert(1)) and [plain](http://x.dev).`;

  assert.equal(
    markdown(text, [], null),
    `<p>See <a href="https://ci.example.com/r?a=1&#38;b=2" rel="noopener noreferrer" target="_blank">the &#34;run&#34;</a> <span class="host">ci.example.com</span>, ` +
      `[bad](javascript:alert(1)) and [plain](http://x.dev).</p>`,
  );
});

test("shows an image as its alt text and host, and loads nothing", () => {
  assert.equal(
    markdown("![the chart](https://img.example.com/c.png)", [], null),
    `<p><span class="img">the chart</span> <span class="host">img.example.com</span></p>`,
  );
});

test("renders a numbered list and a checklist that marks done items", () => {
  const text = "1. Add the table.\n2. Backfill it.\n\n- [x] Migration\n- [ ] Backfill";

  assert.equal(
    markdown(text, [], null),
    "<ol><li>Add the table.</li><li>Backfill it.</li></ol>" +
      `<ul class="checks"><li class="done"><svg class="ico s15" viewBox="0 0 24 24" aria-hidden="true"><path d="M20 6 9 17l-5-5"/></svg><span>Migration</span></li><li><span class="box" aria-hidden="true"></span><span>Backfill</span></li></ul>`,
  );
});

test("nests headings under the section title and renders bold", () => {
  const text = "# Goal\nShip **retries** now.\n## Scope\n#### Files\n#nope";

  assert.equal(
    markdown(text, [], null),
    "<h3>Goal</h3><p>Ship <strong>retries</strong> now.</p><h4>Scope</h4><h5>Files</h5><p>#nope</p>",
  );
});

test("random text yields only the subset's tags and https links", () => {
  const pieces = ["<", ">", "\"", "'", "&", "`", "```", "**", "[", "](", ")", "![", "#", "# ", "- ", "1. ", "[x] ", "\n", "\n\n", " ", "a", "https://h.io/", "javascript:", "<script>", "on=", "ENG-1"];
  const tags = new Set(["p", "br", "ul", "ol", "li", "pre", "code", "strong", "a", "span", "svg", "path", "h3", "h4", "h5"]);
  const links = [{ pattern: String.raw`\b[A-Z]{2,5}-\d+\b`, url: "https://linear.app/i/$0" }];
  let seed = 7;
  const next = () => (seed = (seed * 1103515245 + 12345) % 2147483648);
  for (let round = 0; round < 2000; round += 1) {
    const text = Array.from({ length: next() % 40 }, () => pieces[next() % pieces.length]).join("");
    const html = markdown(text, links, null);
    for (const [, name] of html.matchAll(/<\/?([a-z0-9]+)/g)) assert.ok(tags.has(name), `${name} from ${JSON.stringify(text)}`);
    for (const [, url] of html.matchAll(/href="([^"]*)"/g)) assert.match(url, /^https:\/\//, JSON.stringify(text));
    assert.doesNotMatch(html.replace(/<[^>]*>/g, ""), /[<>"]/, JSON.stringify(text));
  }
});
