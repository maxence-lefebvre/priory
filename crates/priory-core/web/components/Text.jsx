// Agent text and times. Agent text is untrusted: JSX escapes it, and markdown() escapes its own.
import { memo, useMemo } from "react";
import { autolink } from "../links.js";
import { markdown } from "../markdown.js";
import { age } from "../times.js";
import { useLinks } from "../hooks/useData.js";
import { useMinute } from "../hooks/useScreen.js";

/** The one place that writes HTML: markdown() escapes every character of `text`. */
export const Markdown = memo(function Markdown({ text, repo = null }) {
  const links = useLinks();
  const html = useMemo(() => ({ __html: markdown(text, links, repo) }), [text, links, repo]);
  return <div className="md" dangerouslySetInnerHTML={html} />;
});

/** `text` with the factory's autolinks. */
export const Linked = memo(function Linked({ text, repo = null }) {
  const links = useLinks();
  const spans = autolink(String(text ?? ""), links, repo);
  // A span's place in the text is its identity.
  return spans.map((span, at) => (span.url ? <a key={at} className="autolink" href={span.url} rel="noopener noreferrer" target="_blank">{span.text}</a> : span.text));
});

export const Time = memo(function Time({ iso, text }) {
  useMinute();
  return <time dateTime={iso} title={iso ? new Date(iso).toString() : ""}>{text ?? age(iso)}</time>;
});
