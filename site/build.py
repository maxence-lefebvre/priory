#!/usr/bin/env python3
"""Stage the repository's Markdown as an mdBook source folder.

Copies GETTING_STARTED.md and docs/ into site/build/src, writes SUMMARY.md from the docs folders
and their README files, builds the home page from site/home.md, and points links to files outside
the book at GitHub.
"""

import os
import re
import shutil
import sys
from pathlib import Path

REPO = "https://github.com/maxence-lefebvre/priory"
ROOT = Path(__file__).resolve().parent.parent
SITE = ROOT / "site"

# (title, folder, pages placed first); the remaining pages of the folder follow in README link order.
GROUPS = [
    ("Tutorial", None, ["GETTING_STARTED.md"]),
    ("How-to guides", None, [
        "docs/starting-from-a-starter.md",
        "docs/installing-a-release.md",
        "docs/using-the-web-view.md",
        "docs/installing-the-herdr-adapter.md",
        "docs/running-as-services.md",
        "docs/testing-a-factory.md",
        "docs/adding-a-second-machine.md",
        "docs/sharing-build-caches.md",
        "docs/sending-traces-to-opentelemetry.md",
        "docs/evaluating-a-prompt.md",
        "docs/setting-up-a-nightly-dream.md",
        "docs/filtering-and-enriching-events.md",
        "docs/using-priory-from-your-own-session.md",
    ]),
    ("Reference", "docs/reference", ["docs/reference/README.md", "docs/reference/cli.md"]),
    ("How it works", "docs/daemon", [
        "docs/concepts.md",
        "docs/priory-and-other-factories.md",
        "docs/daemon/README.md",
    ]),
    ("The intake", "docs/intake", ["docs/intake/README.md"]),
    ("Adapters", "docs/adapters", [
        "docs/adapters/herdr.md",
        "docs/adapters/github.md",
        "docs/adapters/claude-plugin.md",
    ]),
    ("Decisions", None, ["docs/prd.md"]),
    ("Architecture decisions", "docs/adr", []),
    ("Design notes", "docs/design", []),
]
EXTRA_REFERENCE = ["docs/trace-schema.md"]

LINK = re.compile(r"(!?\[[^\]]*\]\()([^)\s]+)((?:\s+\"[^\"]*\")?\))")
REF_DEF = re.compile(r"^(\s{0,3}\[[^\]]+\]:\s+)(\S+)(.*)$")
FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})")
INDENTED = re.compile(r"^(    |\t)(?![-*+] |\d+\. )")
CODE_SPAN = re.compile(r"`+[^`]*`+")
INCLUDE = re.compile(r"<!-- include (\S+) \"([^\"]+)\" -->")


def title_of(path):
    for line in (ROOT / path).read_text(encoding="utf-8").splitlines():
        if line.startswith("# "):
            return line[2:].strip()
    return Path(path).stem


def readme_order(folder):
    readme = ROOT / folder / "README.md"
    if not readme.exists():
        return []
    names = re.findall(r"\]\(([a-z0-9-]+\.md)[)#]", readme.read_text(encoding="utf-8"))
    return [f"{folder}/{name}" for name in dict.fromkeys(names) if (ROOT / folder / name).exists()]


def chapters():
    """Return [(group title, [page paths])] covering every Markdown page directly in docs folders."""
    placed, groups = set(), []
    for title, folder, first in GROUPS:
        pages = list(first)
        if folder:
            rest = readme_order(folder) + sorted(
                p.relative_to(ROOT).as_posix() for p in (ROOT / folder).glob("*.md")
            )
            pages = list(dict.fromkeys(pages + rest))
        if title == "Reference":
            pages += EXTRA_REFERENCE
        pages = [p for p in pages if p not in placed]
        placed.update(pages)
        groups.append((title, pages))
    loose = sorted(p.relative_to(ROOT).as_posix() for p in (ROOT / "docs").glob("*.md"))
    missing = [p for p in loose if p not in placed]
    if missing:
        sys.exit(f"build.py: place these pages in GROUPS: {', '.join(missing)}")
    return groups


def summary(groups):
    lines = ["# Summary", "", "[Overview](index.md)", ""]
    for title, pages in groups:
        lines += [f"# {title}", ""]
        lines += [f"- [{title_of(p)}]({p})" for p in pages]
        lines.append("")
    return "\n".join(lines)


def rewrite_target(target, source, pages):
    """Map a link found in repository file `source` to a link that works from its staged copy."""
    if re.match(r"^[a-z][a-z0-9+.-]*:", target, re.I) or target.startswith("#"):
        return target
    path, _, fragment = target.partition("#")
    path, _, query = path.partition("?")
    resolved = (ROOT / Path(source).parent / path).resolve()
    try:
        rel = resolved.relative_to(ROOT).as_posix()
    except ValueError:
        return target
    if resolved.is_dir() and (resolved / "README.md").exists() and f"{rel}/README.md" in pages:
        rel = f"{rel}/README.md"
    if rel == "README.md":
        rel = "index.md"
    suffix = (f"?{query}" if query else "") + (f"#{fragment}" if fragment else "")
    copied = rel.startswith("docs/") and resolved.is_file() and not rel.endswith(".md")
    if rel == "index.md" or rel in pages or copied:
        return os.path.relpath(rel, Path(source).parent.as_posix() or ".") + suffix
    kind = "tree" if resolved.is_dir() else "blob"
    return f"{REPO}/{kind}/main/{rel}{suffix}"


def rewrite_links(text, source, pages):
    """Rewrite link targets outside code: fenced blocks, indented blocks and code spans."""
    out, fence = [], None
    swap = lambda m: m[1] + rewrite_target(m[2], source, pages) + m[3]
    for line in text.split("\n"):
        opener = FENCE.match(line)
        if fence is None and opener:
            fence = opener[1]
        elif fence and opener and opener[1][0] == fence[0] and len(opener[1]) >= len(fence) and not line.strip(" `~"):
            fence = None
        elif fence is None and not INDENTED.match(line):
            spans = [m.span() for m in CODE_SPAN.finditer(line)]
            inside = lambda m: any(a <= m.start() < b for a, b in spans)
            line = LINK.sub(lambda m: m[0] if inside(m) else swap(m), line)
            line = REF_DEF.sub(swap, line)
        out.append(line)
    return "\n".join(out)


def section(path, heading):
    """Return the body under `## heading` in a repository file, or the intro before the first ##."""
    lines = (ROOT / path).read_text(encoding="utf-8").splitlines()
    if heading == "intro":
        start = next(i for i, l in enumerate(lines) if l.startswith("</h1>")) + 1
        # The README's tagline is the site's hero already: skip its paragraph.
        while start < len(lines) and not lines[start].strip():
            start += 1
        if start < len(lines) and lines[start].startswith("<!-- tagline"):
            while start < len(lines) and lines[start].strip():
                start += 1
    else:
        start = lines.index(f"## {heading}") + 1
    end = next((i for i in range(start, len(lines)) if lines[i].startswith("## ")), len(lines))
    return "\n".join(lines[start:end]).strip()


def home(pages):
    template = (SITE / "home.md").read_text(encoding="utf-8")
    return INCLUDE.sub(lambda m: rewrite_links(section(m[1], m[2]), m[1], pages), template)


def main():
    out = SITE / "build" / "src"
    if any(p.is_symlink() for p in (SITE / "build", out)):
        sys.exit("build.py: site/build or site/build/src is a symlink; remove it before staging")
    if out.exists():
        shutil.rmtree(out)
    shutil.copytree(ROOT / "docs", out / "docs")
    groups = chapters()
    pages = {p for _, ps in groups for p in ps}
    for page in pages:
        text = (ROOT / page).read_text(encoding="utf-8")
        (out / page).write_text(rewrite_links(text, page, pages), encoding="utf-8")
    for unlisted in out.joinpath("docs").rglob("*.md"):
        if unlisted.relative_to(out).as_posix() not in pages:
            unlisted.unlink()
    (out / "SUMMARY.md").write_text(summary(groups), encoding="utf-8")
    (out / "index.md").write_text(home(pages), encoding="utf-8")
    assets = out / "assets"
    shutil.copytree(ROOT / "design" / "fonts", assets / "fonts", ignore=shutil.ignore_patterns("*.md"))
    for name in ("lockup.svg", "lockup-dark.svg", "favicon.svg", "apple-touch-icon.png"):
        shutil.copy(ROOT / "design" / "logo" / name, assets / name)
    print(f"staged {len(pages) + 1} pages in {out.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
