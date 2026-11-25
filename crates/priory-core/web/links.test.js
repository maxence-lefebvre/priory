import { test } from "node:test";
import assert from "node:assert/strict";
import { autolink, linked } from "./links.js";

const LINKS = [
  { pattern: String.raw`\b([A-Z]{2,5}-\d+)\b`, url: "https://linear.app/acme/issue/$1" },
  { pattern: String.raw`\B#(\d+)\b`, url: "https://github.com/{repo}/issues/$1" },
];

test("links keys in the escaped text and never inside markup the text spells", () => {
  const html = linked(`<a href="#12" title="ENG-7">it's #4</a>`, LINKS, "acme/app");

  assert.equal(
    html,
    `&#60;a href=&#34;<a class="autolink" href="https://github.com/acme/app/issues/12" rel="noopener noreferrer" target="_blank">#12</a>&#34; ` +
      `title=&#34;<a class="autolink" href="https://linear.app/acme/issue/ENG-7" rel="noopener noreferrer" target="_blank">ENG-7</a>&#34;&#62;` +
      `it&#39;s <a class="autolink" href="https://github.com/acme/app/issues/4" rel="noopener noreferrer" target="_blank">#4</a>&#60;/a&#62;`,
  );
});

test("drops a template that is not https, and a {repo} template when the task has no repository", () => {
  const links = [{ pattern: String.raw`\bdoc-(\d+)\b`, url: "javascript:alert($1)" }, ...LINKS];

  assert.equal(linked("doc-1 and #2", links, null), "doc-1 and #2");
});

test("percent-encodes what a group captured", () => {
  const links = [{ pattern: String.raw`\bsee:(\S+)`, url: "https://example.com/q?x=$1" }];

  assert.equal(
    linked(`see:a"b&c`, links, null),
    `<a class="autolink" href="https://example.com/q?x=a%22b%26c" rel="noopener noreferrer" target="_blank">see:a&#34;b&#38;c</a>`,
  );
});

test("an empty match does not hide a later match", () => {
  const links = [{ pattern: String.raw`\d*`, url: "https://example.com/$0" }];

  assert.equal(
    linked("abc123", links, null),
    `abc<a class="autolink" href="https://example.com/123" rel="noopener noreferrer" target="_blank">123</a>`,
  );
});

test("an empty match before an astral character moves past the whole character", () => {
  const links = [{ pattern: String.raw`\d*`, url: "https://example.com/$0" }];

  assert.equal(
    linked("😀123", links, null),
    `😀<a class="autolink" href="https://example.com/123" rel="noopener noreferrer" target="_blank">123</a>`,
  );
});

test("links a Desk card's bare URL and its owner/repo#N, and leaves the prose around them", () => {
  const rule = [{ pattern: String.raw`\b([\w.-]+)/([\w.-]+)#(\d+)\b`, url: "https://github.com/$1/$2/issues/$3" }];
  const text = "priory: landed wrong: acme/shop#212 of task 7 is red on dev: deploy cancelled https://github.com/acme/shop/actions/runs/1001.";

  assert.equal(
    linked(text, rule, null),
    `priory: landed wrong: <a class="autolink" href="https://github.com/acme/shop/issues/212" rel="noopener noreferrer" target="_blank">acme/shop#212</a>` +
      ` of task 7 is red on dev: deploy cancelled ` +
      `<a class="autolink" href="https://github.com/acme/shop/actions/runs/1001" rel="noopener noreferrer" target="_blank">https://github.com/acme/shop/actions/runs/1001</a>.`,
  );
});

test("a bare URL never takes an http target or markup around it, and escapes its href", () => {
  assert.equal(linked("http://x.dev", [], null), "http://x.dev");
  assert.equal(
    linked(`<https://x.dev/a?b=1&c="2">`, [], null),
    `&#60;<a class="autolink" href="https://x.dev/a?b=1&#38;c=" rel="noopener noreferrer" target="_blank">https://x.dev/a?b=1&#38;c=</a>&#34;2&#34;&#62;`,
  );
});

test("a bare URL ends at a backtick or ]", () => {
  assert.equal(
    linked("see `https://ci.example.com/x`", [], null),
    'see `<a class="autolink" href="https://ci.example.com/x" rel="noopener noreferrer" target="_blank">https://ci.example.com/x</a>`',
  );
});

test("a long run of prose after a URL is left out in linear time", () => {
  for (const tail of [")", ".)", "."]) {
    const started = performance.now();
    const [span, rest] = autolink(`https://a.dev/x${tail.repeat(65_536 / tail.length)}`, [], null);
    assert.equal(span.url, "https://a.dev/x");
    assert.equal(rest.text.length, 65_536);
    assert.ok(performance.now() - started < 1000, tail);
  }
  const dotted = `https://a.dev/${".".repeat(65_536)}x`;
  const started = performance.now();
  assert.equal(autolink(dotted, [], null).at(-1)?.text, dotted);
  assert.ok(performance.now() - started < 1000, "dots");
  assert.deepEqual(autolink("(https://a.dev/b_(c)).", [], null).map((span) => span.text), ["(", "https://a.dev/b_(c)", ")."]);
});
